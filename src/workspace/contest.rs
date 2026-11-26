use crate::client::model::{Contest, ContestType};
use crate::validation::validate_atcoder_identifier;
use crate::workspace::problem::{PROBLEM_JSON, write_problem_to};
use crate::workspace::template::TemplateData;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct ContestIndex<'a> {
    id: &'a str,
    contest_type: &'a ContestType,
    title: &'a str,
    problems: BTreeMap<&'a str, String>,
}

pub(crate) fn save_contest_to(
    base: &Path,
    contest: &Contest,
    template: Option<&TemplateData>,
) -> Result<PathBuf> {
    validate_atcoder_identifier(&contest.id, "contest ID")?;
    let contest_dir = base.join(&contest.id);
    if contest_dir.exists() {
        bail!(
            "Contest directory '{}' already exists; refusing to overwrite solutions.",
            contest_dir.display()
        );
    }

    let staging = tempfile::Builder::new()
        .prefix(".ackit-contest-")
        .tempdir_in(base)
        .context("Failed to create contest staging directory.")?;
    let staging_path = staging.path();
    let mut index = ContestIndex {
        id: &contest.id,
        contest_type: &contest.contest_type,
        title: &contest.title,
        problems: BTreeMap::new(),
    };
    for (label, problem) in &contest.problems {
        validate_atcoder_identifier(label, "problem label")?;
        let directory = label.to_ascii_lowercase();
        write_problem_to(&staging_path.join(&directory), problem, template)?;
        index
            .problems
            .insert(label, format!("{directory}/{PROBLEM_JSON}"));
    }

    {
        let mut contest_json = io::BufWriter::new(
            fs::File::create(staging_path.join("contest.json"))
                .context("Failed to create contest.json.")?,
        );
        serde_json::to_writer_pretty(&mut contest_json, &index)
            .context("Failed to write contest.json.")?;
        contest_json
            .flush()
            .context("Failed to flush contest.json.")?;
    }

    let staging_path = staging.keep();
    if let Err(error) = fs::rename(&staging_path, &contest_dir) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(error).context("Failed to install contest directory.");
    }
    Ok(contest_dir)
}

pub(crate) fn find_contest_root_from(start: &Path) -> Result<PathBuf> {
    let mut current = start.to_path_buf();
    loop {
        if current.join("contest.json").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            bail!("Failed to find `contest.json` directory.");
        }
    }
}

pub(crate) fn load_legacy_contest_from(path: &Path) -> Result<Contest> {
    let file =
        fs::File::open(path).with_context(|| format!("Failed to open '{}'.", path.display()))?;
    serde_json::from_reader(io::BufReader::new(file))
        .with_context(|| format!("Failed to parse '{}'.", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::template::StoredTemplateConfig;

    fn fixture_contest() -> Contest {
        serde_json::from_str(include_str!("../../tests/fixtures/json/contest.json")).unwrap()
    }

    fn fixture_contest_with_two_problems() -> Contest {
        let mut contest = fixture_contest();
        let mut problem = contest.problems["A"].clone();
        problem.id = "abc999_b".into();
        problem.label = "B".into();
        problem.url = "https://atcoder.jp/contests/abc999/tasks/abc999_b".into();
        contest.problems.insert("B".into(), problem);
        contest
    }

    fn template_data(path: &Path) -> TemplateData {
        TemplateData {
            template_path: path.to_path_buf(),
            name: "python".into(),
            config: StoredTemplateConfig {
                name: "python".into(),
                submit_file: "main.py".into(),
                language_id: 5078,
                exec_command: vec!["python".into(), "main.py".into()],
                compile_command: None,
                pre_submit: None,
            },
            is_default: true,
        }
    }

    fn create_template_files(root: &Path) {
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(
            root.join("template.json"),
            include_str!("../../tests/fixtures/json/template_legacy.json"),
        )
        .unwrap();
        fs::write(root.join("main.py"), "print(input())\n").unwrap();
        fs::write(root.join(".env"), "LOCAL=1\n").unwrap();
        fs::write(root.join("nested/data.txt"), "data\n").unwrap();
    }

    #[test]
    fn saves_contest_index_and_all_problems_with_template() {
        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        fs::write(template_root.join("problem.json"), "template data").unwrap();
        let contest = fixture_contest_with_two_problems();

        let root =
            save_contest_to(temp.path(), &contest, Some(&template_data(&template_root))).unwrap();
        let index: serde_json::Value =
            serde_json::from_reader(fs::File::open(root.join("contest.json")).unwrap()).unwrap();
        assert_eq!(
            index,
            serde_json::json!({
                "id": "abc999",
                "contest_type": "ABC",
                "title": contest.title,
                "problems": {"A": "a/problem.json", "B": "b/problem.json"},
            })
        );
        for (label, problem) in &contest.problems {
            let path = root.join(index["problems"][label.as_str()].as_str().unwrap());
            let workspace =
                crate::workspace::problem::ProblemWorkspace::discover_from(path.parent().unwrap())
                    .unwrap();
            assert_eq!(workspace.problem(), problem);
            assert!(workspace.submit_path().is_file());
        }
    }

    #[test]
    fn saves_every_problem_without_a_template() {
        let temp = tempfile::tempdir().unwrap();
        let contest = fixture_contest_with_two_problems();
        let root = save_contest_to(temp.path(), &contest, None).unwrap();

        for (label, problem) in &contest.problems {
            let directory = root.join(label.to_ascii_lowercase());
            let entries: Vec<_> = fs::read_dir(&directory)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(entries, [PROBLEM_JSON]);
            let saved: crate::client::model::Problem =
                serde_json::from_reader(fs::File::open(directory.join(PROBLEM_JSON)).unwrap())
                    .unwrap();
            assert_eq!(&saved, problem);
        }
    }

    #[test]
    fn finds_nearest_contest_root_without_changing_cwd() {
        let temp = tempfile::tempdir().unwrap();
        let contest_root = temp.path().join("abc999");
        let nested = contest_root.join("a/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(contest_root.join("contest.json"), "{}").unwrap();
        assert_eq!(find_contest_root_from(&nested).unwrap(), contest_root);
    }

    #[test]
    fn refuses_to_overwrite_an_existing_contest_directory() {
        let temp = tempfile::tempdir().unwrap();
        let contest = fixture_contest();
        fs::create_dir(temp.path().join("abc999")).unwrap();
        assert!(save_contest_to(temp.path(), &contest, None).is_err());
    }

    #[test]
    fn copies_all_template_files_when_ackitignore_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);

        save_contest_to(
            temp.path(),
            &fixture_contest(),
            Some(&template_data(&template_root)),
        )
        .unwrap();

        let problem = temp.path().join("abc999/a");
        assert!(problem.join("template.json").is_file());
        assert!(problem.join("main.py").is_file());
        assert!(problem.join(".env").is_file());
        assert!(problem.join("nested/data.txt").is_file());
    }

    #[test]
    fn applies_ackitignore_patterns_and_preserves_required_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        fs::create_dir_all(template_root.join("target")).unwrap();
        fs::write(template_root.join("target/cache.bin"), "cache\n").unwrap();
        fs::write(template_root.join("scratch.tmp"), "temporary\n").unwrap();
        fs::write(template_root.join("notes.txt"), "ignored\n").unwrap();
        fs::write(template_root.join("README.txt"), "kept\n").unwrap();
        fs::write(
            template_root.join(".ackitignore"),
            "# generated files\ntarget/\n*.tmp\n*.txt\n!README.txt\n*.json\n!.ackitignore\n",
        )
        .unwrap();

        save_contest_to(
            temp.path(),
            &fixture_contest_with_two_problems(),
            Some(&template_data(&template_root)),
        )
        .unwrap();

        for label in ["a", "b"] {
            let problem = temp.path().join("abc999").join(label);
            assert!(problem.join("main.py").is_file());
            assert!(problem.join("README.txt").is_file());
            assert!(problem.join("template.json").is_file());
            assert!(!problem.join(".ackitignore").exists());
            assert!(!problem.join("target").exists());
            assert!(!problem.join("scratch.tmp").exists());
            assert!(!problem.join("notes.txt").exists());
            assert!(!problem.join("nested/data.txt").exists());
            crate::workspace::problem::ProblemWorkspace::discover_from(&problem).unwrap();
        }
    }

    #[test]
    fn rejects_non_file_ackitignore_without_installing_the_contest() {
        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        fs::create_dir(template_root.join(".ackitignore")).unwrap();

        assert!(
            save_contest_to(
                temp.path(),
                &fixture_contest(),
                Some(&template_data(&template_root)),
            )
            .is_err()
        );
        assert!(!temp.path().join("abc999").exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unignored_template_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, "outside\n").unwrap();
        symlink(&outside, template_root.join("linked.txt")).unwrap();

        assert!(
            save_contest_to(
                temp.path(),
                &fixture_contest(),
                Some(&template_data(&template_root)),
            )
            .is_err()
        );
        assert!(!temp.path().join("abc999").exists());
    }

    #[cfg(unix)]
    #[test]
    fn skips_ignored_template_symlinks() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, "outside\n").unwrap();
        symlink(&outside, template_root.join("linked.txt")).unwrap();
        fs::write(template_root.join(".ackitignore"), "linked.txt\n").unwrap();

        save_contest_to(
            temp.path(),
            &fixture_contest(),
            Some(&template_data(&template_root)),
        )
        .unwrap();
        assert!(!temp.path().join("abc999/a/linked.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_inspect_symlinks_inside_ignored_directories() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("python-template");
        create_template_files(&template_root);
        let generated = template_root.join("generated");
        fs::create_dir(&generated).unwrap();
        let outside = temp.path().join("outside.txt");
        fs::write(&outside, "outside\n").unwrap();
        symlink(&outside, generated.join("linked.txt")).unwrap();
        fs::write(template_root.join(".ackitignore"), "generated/\n").unwrap();

        save_contest_to(
            temp.path(),
            &fixture_contest(),
            Some(&template_data(&template_root)),
        )
        .unwrap();
        assert!(!temp.path().join("abc999/a/generated").exists());
    }
}
