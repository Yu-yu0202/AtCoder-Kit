use crate::client::model::{Contest, Problem};
use crate::workspace::contest::{find_contest_root_from, load_contest_from};
use crate::workspace::template::{TemplateConfig, load_template_config_from};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(crate) struct ProblemWorkspace {
    problem_dir: PathBuf,
    template: TemplateConfig,
    contest: Contest,
    problem_key: String,
}

impl ProblemWorkspace {
    pub(crate) fn discover_from(start: &Path) -> Result<Self> {
        let problem_dir = start.to_path_buf();
        let template = load_template_config_from(&problem_dir.join("template.json"))?;
        let contest_root = find_contest_root_from(&problem_dir)?;
        let contest = load_contest_from(&contest_root.join("contest.json"))?;
        let problem_key = problem_dir
            .file_name()
            .and_then(|name| name.to_str())
            .context("Problem directory name must be valid UTF-8.")?
            .to_uppercase();
        contest
            .problems
            .get(&problem_key)
            .with_context(|| format!("Failed to get problem {problem_key} from contest."))?;

        Ok(Self {
            problem_dir,
            template,
            contest,
            problem_key,
        })
    }

    pub(crate) fn problem_dir(&self) -> &Path {
        &self.problem_dir
    }

    pub(crate) fn template(&self) -> &TemplateConfig {
        &self.template
    }

    pub(crate) fn contest(&self) -> &Contest {
        &self.contest
    }

    pub(crate) fn problem(&self) -> &Problem {
        &self.contest.problems[&self.problem_key]
    }

    pub(crate) fn submit_path(&self) -> PathBuf {
        self.problem_dir.join(&self.template.submit_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn discovers_workspace_from_an_explicit_path() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("abc999");
        let problem = root.join("a");
        fs::create_dir_all(&problem).unwrap();
        fs::write(
            root.join("contest.json"),
            include_str!("../../tests/fixtures/json/contest.json"),
        )
        .unwrap();
        fs::write(
            problem.join("template.json"),
            include_str!("../../tests/fixtures/json/template_legacy.json"),
        )
        .unwrap();

        let workspace = ProblemWorkspace::discover_from(&problem).unwrap();
        assert_eq!(workspace.problem().id, "abc999_a");
        assert_eq!(workspace.contest().id, "abc999");
        assert_eq!(workspace.submit_path(), problem.join("main.py"));
    }
}
