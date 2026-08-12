use crate::client::model::Contest;
use crate::validation::validate_atcoder_identifier;
use crate::workspace::template::TemplateData;
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn copy_recursive(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("Failed to inspect '{}'.", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Template symlinks are not supported: '{}'.",
            source.display()
        );
    }
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("Unsupported template entry: '{}'.", source.display());
    }

    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_recursive(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
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
    let mut contest_json = io::BufWriter::new(
        fs::File::create(staging_path.join("contest.json"))
            .context("Failed to create contest.json.")?,
    );
    serde_json::to_writer_pretty(&mut contest_json, contest)
        .context("Failed to write contest.json.")?;
    contest_json
        .flush()
        .context("Failed to flush contest.json.")?;
    drop(contest_json);

    if let Some(template) = template {
        for label in contest.problems.keys() {
            validate_atcoder_identifier(label, "problem label")?;
            let problem_dir = staging_path.join(label.to_lowercase());
            fs::create_dir_all(&problem_dir).context("Failed to create problem directory.")?;
            copy_recursive(&template.template_path, &problem_dir)
                .context("Failed to copy template.")?;
        }
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

pub(crate) fn load_contest_from(path: &Path) -> Result<Contest> {
    let file =
        fs::File::open(path).with_context(|| format!("Failed to open '{}'.", path.display()))?;
    serde_json::from_reader(io::BufReader::new(file))
        .with_context(|| format!("Failed to parse '{}'.", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_existing_contest_json_contract() {
        let fixture = include_str!("../../tests/fixtures/json/contest.json");
        let contest: Contest = serde_json::from_str(fixture).unwrap();
        assert_eq!(contest.id, "abc999");
        assert_eq!(
            serde_json::to_value(&contest).unwrap(),
            serde_json::from_str::<serde_json::Value>(fixture).unwrap()
        );
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
        let contest: Contest =
            serde_json::from_str(include_str!("../../tests/fixtures/json/contest.json")).unwrap();
        fs::create_dir(temp.path().join("abc999")).unwrap();
        assert!(save_contest_to(temp.path(), &contest, None).is_err());
    }
}
