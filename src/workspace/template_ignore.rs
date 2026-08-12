use anyhow::{Context, Result, bail};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub(crate) const IGNORE_FILE_NAME: &str = ".ackitignore";
const TEMPLATE_CONFIG_NAME: &str = "template.json";

pub(crate) struct TemplateIgnore {
    root: PathBuf,
    matcher: Gitignore,
}

impl TemplateIgnore {
    pub(crate) fn load(root: &Path) -> Result<Self> {
        let ignore_path = root.join(IGNORE_FILE_NAME);
        match fs::symlink_metadata(&ignore_path) {
            Ok(metadata) if !metadata.is_file() => {
                bail!(
                    "Template ignore file must be a regular file: '{}'.",
                    ignore_path.display()
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Self {
                    root: root.to_path_buf(),
                    matcher: GitignoreBuilder::new(root)
                        .build()
                        .context("Failed to initialize template ignore matcher.")?,
                });
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("Failed to inspect '{}'.", ignore_path.display()));
            }
        }

        let mut builder = GitignoreBuilder::new(root);
        if let Some(error) = builder.add(&ignore_path) {
            return Err(error)
                .with_context(|| format!("Failed to read '{}'.", ignore_path.display()));
        }
        let matcher = builder
            .build()
            .with_context(|| format!("Failed to parse '{}'.", ignore_path.display()))?;
        Ok(Self {
            root: root.to_path_buf(),
            matcher,
        })
    }

    pub(crate) fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        let relative = path.strip_prefix(&self.root).unwrap_or(path);
        if relative.as_os_str().is_empty() {
            return false;
        }
        if relative == Path::new(IGNORE_FILE_NAME) {
            return true;
        }
        if relative == Path::new(TEMPLATE_CONFIG_NAME) {
            return false;
        }
        self.matcher.matched(path, is_dir).is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_ignore_file_matches_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let matcher = TemplateIgnore::load(temp.path()).unwrap();
        assert!(!matcher.is_ignored(&temp.path().join("main.rs"), false));
    }

    #[test]
    fn reserves_control_files() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(IGNORE_FILE_NAME),
            "*.json\n!.ackitignore\n",
        )
        .unwrap();
        let matcher = TemplateIgnore::load(temp.path()).unwrap();

        assert!(matcher.is_ignored(&temp.path().join(IGNORE_FILE_NAME), false));
        assert!(!matcher.is_ignored(&temp.path().join(TEMPLATE_CONFIG_NAME), false));
        assert!(matcher.is_ignored(&temp.path().join("data.json"), false));
    }

    #[test]
    fn rejects_non_file_ignore_path() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join(IGNORE_FILE_NAME)).unwrap();
        assert!(TemplateIgnore::load(temp.path()).is_err());
    }

    #[test]
    fn rejects_invalid_patterns() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(IGNORE_FILE_NAME), "[z-a]\n").unwrap();
        assert!(TemplateIgnore::load(temp.path()).is_err());
    }

    #[test]
    fn matches_case_sensitively() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join(IGNORE_FILE_NAME), "*.TMP\n").unwrap();
        let matcher = TemplateIgnore::load(temp.path()).unwrap();

        assert!(matcher.is_ignored(&temp.path().join("result.TMP"), false));
        assert!(!matcher.is_ignored(&temp.path().join("result.tmp"), false));
    }
}
