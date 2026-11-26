use crate::client::model::Problem;
use crate::validation::validate_atcoder_identifier;
use crate::workspace::contest::{find_contest_root_from, load_legacy_contest_from};
use crate::workspace::template::{TemplateConfig, TemplateData, load_template_config_from};
use crate::workspace::template_ignore::TemplateIgnore;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(crate) const PROBLEM_JSON: &str = "problem.json";

pub(crate) fn save_problem_to(
    base: &Path,
    problem: &Problem,
    template: Option<&TemplateData>,
) -> Result<PathBuf> {
    validate_atcoder_identifier(&problem.id, "problem ID")?;
    let destination = base.join(&problem.id);
    let staging = tempfile::Builder::new()
        .prefix(".ackit-problem-")
        .tempdir_in(base)
        .context("Failed to create problem staging directory.")?;
    let staged_problem = staging.path().join(&problem.id);
    write_problem_to(&staged_problem, problem, template)?;

    match install_problem_directory(&staged_problem, &destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => bail!(
            "Problem directory '{}' already exists; refusing to overwrite solutions.",
            destination.display()
        ),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to install problem directory '{}'.",
                    destination.display()
                )
            });
        }
    }
    Ok(destination)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox"
))]
fn install_problem_directory(source: &Path, destination: &Path) -> io::Result<()> {
    use rustix::fs::{CWD, RenameFlags, renameat_with};

    // A plain rename can replace an empty directory created while staging.
    renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE)?;
    Ok(())
}

#[cfg(windows)]
fn install_problem_directory(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

    let to_wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain([0])
            .collect::<Vec<_>>()
    };
    let source = to_wide(source);
    let destination = to_wide(destination);
    // Both paths are NUL-terminated and live for the call; flags of 0 forbid replacement.
    if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_vendor = "apple",
    target_os = "redox",
    windows
)))]
fn install_problem_directory(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Atomic directory installation without replacement is unavailable on this platform.",
    ))
}

fn copy_recursive(source: &Path, destination: &Path, ignore: &TemplateIgnore) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to inspect '{}'.", source.display()))?;
    if ignore.is_ignored(source, metadata.is_dir()) {
        return Ok(());
    }
    if metadata.file_type().is_symlink() {
        bail!(
            "Template symlinks are not supported: '{}'.",
            source.display()
        );
    }
    if metadata.is_file() {
        fs::copy(source, destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("Unsupported template entry: '{}'.", source.display());
    }

    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_recursive(&entry.path(), &destination.join(entry.file_name()), ignore)?;
    }
    Ok(())
}

// The caller can supply a destination inside its staging directory for atomic installation.
pub(crate) fn write_problem_to(
    destination: &Path,
    problem: &Problem,
    template: Option<&TemplateData>,
) -> Result<()> {
    fs::create_dir(destination).with_context(|| {
        format!(
            "Failed to create problem directory '{}'.",
            destination.display()
        )
    })?;
    if let Some(template) = template {
        let ignore = TemplateIgnore::load(&template.template_path)
            .context("Failed to load template ignore rules.")?;
        copy_recursive(&template.template_path, destination, &ignore)
            .context("Failed to copy template.")?;
    }

    let path = destination.join(PROBLEM_JSON);
    let mut writer = io::BufWriter::new(
        fs::File::create_new(&path)
            .with_context(|| format!("Failed to create '{}'.", path.display()))?,
    );
    serde_json::to_writer_pretty(&mut writer, problem)
        .with_context(|| format!("Failed to write '{}'.", path.display()))?;
    writer
        .flush()
        .with_context(|| format!("Failed to flush '{}'.", path.display()))
}

fn load_problem_from(problem_dir: &Path) -> Result<Problem> {
    let path = problem_dir.join(PROBLEM_JSON);
    match fs::symlink_metadata(&path) {
        Ok(_) => {
            let file = fs::File::open(&path)
                .with_context(|| format!("Failed to open '{}'.", path.display()))?;
            serde_json::from_reader(io::BufReader::new(file))
                .with_context(|| format!("Failed to parse '{}'.", path.display()))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let contest_root = find_contest_root_from(problem_dir)?;
            let mut contest = load_legacy_contest_from(&contest_root.join("contest.json"))?;
            let problem_key = problem_dir
                .file_name()
                .and_then(|name| name.to_str())
                .context("Problem directory name must be valid UTF-8.")?
                .to_uppercase();
            contest
                .problems
                .remove(&problem_key)
                .with_context(|| format!("Failed to get problem {problem_key} from contest."))
        }
        Err(error) => {
            Err(error).with_context(|| format!("Failed to inspect '{}'.", path.display()))
        }
    }
}

pub(crate) struct ProblemWorkspace {
    problem_dir: PathBuf,
    template: TemplateConfig,
    problem: Problem,
}

impl ProblemWorkspace {
    pub(crate) fn discover_from(start: &Path) -> Result<Self> {
        let problem = load_problem_from(start)?;
        let template = load_template_config_from(&start.join("template.json"))?;

        Ok(Self {
            problem_dir: start.to_path_buf(),
            template,
            problem,
        })
    }

    pub(crate) fn problem_dir(&self) -> &Path {
        &self.problem_dir
    }

    pub(crate) fn template(&self) -> &TemplateConfig {
        &self.template
    }

    pub(crate) fn problem(&self) -> &Problem {
        &self.problem
    }

    pub(crate) fn submit_path(&self) -> PathBuf {
        self.problem_dir.join(&self.template.submit_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_CONTEST: &str = include_str!("../../tests/fixtures/json/contest.json");
    const TEMPLATE: &str = include_str!("../../tests/fixtures/json/template_legacy.json");

    fn fixture_problem() -> Problem {
        let mut contest: crate::client::model::Contest =
            serde_json::from_str(LEGACY_CONTEST).unwrap();
        contest.problems.remove("A").unwrap()
    }

    fn create_legacy_workspace(base: &Path) -> PathBuf {
        let root = base.join("abc999");
        let problem = root.join("a");
        fs::create_dir_all(&problem).unwrap();
        fs::write(root.join("contest.json"), LEGACY_CONTEST).unwrap();
        fs::write(problem.join("template.json"), TEMPLATE).unwrap();
        problem
    }

    #[test]
    fn discovers_legacy_workspace_from_an_explicit_path() {
        let temp = tempfile::tempdir().unwrap();
        let problem = create_legacy_workspace(temp.path());
        let workspace = ProblemWorkspace::discover_from(&problem).unwrap();
        assert_eq!(workspace.problem().id, "abc999_a");
        assert_eq!(workspace.problem().contest_id, "abc999");
        assert_eq!(workspace.submit_path(), problem.join("main.py"));
    }

    #[test]
    fn saves_standalone_workspace_with_template_and_no_contest_json() {
        let temp = tempfile::tempdir().unwrap();
        let template_root = temp.path().join("template");
        fs::create_dir(&template_root).unwrap();
        fs::write(template_root.join("template.json"), TEMPLATE).unwrap();
        fs::write(template_root.join("main.py"), "print(input())\n").unwrap();
        let template = TemplateData {
            template_path: template_root,
            ..TemplateData::default()
        };
        let problem = fixture_problem();

        let directory = save_problem_to(temp.path(), &problem, Some(&template)).unwrap();
        assert_eq!(directory, temp.path().join("abc999_a"));
        assert!(!temp.path().join("contest.json").exists());
        let workspace = ProblemWorkspace::discover_from(&directory).unwrap();
        assert_eq!(workspace.problem(), &problem);
        assert_eq!(workspace.submit_path(), directory.join("main.py"));
        assert_eq!(
            fs::read_to_string(workspace.submit_path()).unwrap(),
            "print(input())\n"
        );
    }

    #[test]
    fn saves_only_problem_metadata_without_template() {
        let temp = tempfile::tempdir().unwrap();
        let problem = fixture_problem();
        let directory = save_problem_to(temp.path(), &problem, None).unwrap();

        assert_eq!(directory, temp.path().join("abc999_a"));
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(load_problem_from(&directory).unwrap(), problem);
    }

    #[test]
    fn refuses_to_replace_existing_problem_directory() {
        let temp = tempfile::tempdir().unwrap();
        let problem = fixture_problem();
        let directory = temp.path().join(&problem.id);
        fs::create_dir(&directory).unwrap();
        assert!(save_problem_to(temp.path(), &problem, None).is_err());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 0);

        fs::write(directory.join("main.py"), "solution").unwrap();
        fs::write(directory.join(PROBLEM_JSON), "metadata").unwrap();
        assert!(save_problem_to(temp.path(), &problem, None).is_err());
        assert_eq!(fs::read(directory.join("main.py")).unwrap(), b"solution");
        assert_eq!(fs::read(directory.join(PROBLEM_JSON)).unwrap(), b"metadata");
        assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
    }

    #[test]
    fn reports_invalid_problem_json_even_when_legacy_metadata_exists() {
        let temp = tempfile::tempdir().unwrap();
        let problem = create_legacy_workspace(temp.path());
        fs::write(problem.join(PROBLEM_JSON), "{").unwrap();

        let Err(error) = ProblemWorkspace::discover_from(&problem) else {
            panic!("Invalid problem.json should fail discovery.");
        };
        assert!(error.to_string().contains("Failed to parse"));
        assert!(error.to_string().contains(PROBLEM_JSON));
    }
}
