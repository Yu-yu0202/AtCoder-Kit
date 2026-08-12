use crate::application::sample::{TestResult, run_sample_tests};
use crate::client::AtCoderClient;
use crate::workspace::command::{CommandInput, CommandRunner};
use crate::workspace::problem::ProblemWorkspace;
use anyhow::{Context, Result, bail};
use std::path::Path;
use std::time::Duration;

const PRE_SUBMIT_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn normalize_source_to_crlf(source: &str) -> String {
    let mut normalized = String::with_capacity(source.len());
    for line in source.lines() {
        normalized.push_str(line.strip_suffix('\r').unwrap_or(line));
        normalized.push_str("\r\n");
    }
    normalized
}

async fn read_source_crlf(path: &Path, problem_dir: &Path) -> Result<String> {
    let canonical_root = tokio::fs::canonicalize(problem_dir)
        .await
        .with_context(|| format!("Failed to resolve '{}'.", problem_dir.display()))?;
    let canonical_path = tokio::fs::canonicalize(path)
        .await
        .with_context(|| format!("Failed to resolve source file '{}'.", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) {
        bail!("Submit file must stay inside the problem directory.");
    }
    let source = tokio::fs::read_to_string(&canonical_path)
        .await
        .with_context(|| format!("Failed to read source file '{}'.", canonical_path.display()))?;
    Ok(normalize_source_to_crlf(&source))
}

pub(crate) async fn run_pre_submit(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
) -> Result<()> {
    if let Some(pre_submit) = &workspace.template().pre_submit {
        let output = runner
            .run(
                pre_submit,
                workspace.problem_dir(),
                CommandInput::Null,
                PRE_SUBMIT_TIMEOUT,
            )
            .await?;
        if !output.success {
            let timeout_message = if output.timed_out {
                format!(
                    "\ncommand timed out after {} seconds",
                    PRE_SUBMIT_TIMEOUT.as_secs()
                )
            } else {
                String::new()
            };
            bail!(
                "Failed to run pre-submit command.{timeout_message}\nexit code: {}\nstdout:\n{}\nstderr:\n{}",
                output.exit_code.unwrap_or(-1),
                output.stdout,
                output.stderr
            );
        }
    }
    Ok(())
}

pub(crate) async fn prepare_solution(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    no_test: bool,
) -> Result<String> {
    run_pre_submit(workspace, runner).await?;
    if !no_test {
        let results = run_sample_tests(workspace, runner).await?;
        if results.iter().any(TestResult::is_failed) {
            bail!("Test failed. Please fix the issues and try submitting again.");
        }
    }
    read_source_crlf(&workspace.submit_path(), workspace.problem_dir()).await
}

pub(crate) async fn submit_prepared_solution(
    workspace: &ProblemWorkspace,
    client: &AtCoderClient,
    source: String,
) -> Result<String> {
    client
        .submit_solution(
            &workspace.contest().id,
            workspace.problem(),
            workspace.template().language_id,
            source,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::command::{CommandOutput, CommandSpec};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        calls: Mutex<Vec<Vec<String>>>,
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(
            &self,
            command: &CommandSpec,
            _cwd: &Path,
            _input: CommandInput,
            _timeout: Duration,
        ) -> Result<CommandOutput> {
            self.calls.lock().unwrap().push(command.words());
            Ok(self.outputs.lock().unwrap().pop_front().unwrap())
        }
    }

    impl FakeRunner {
        fn with_outputs(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    fn output(success: bool, stdout: &str) -> CommandOutput {
        CommandOutput {
            success,
            timed_out: false,
            exit_code: Some(if success { 0 } else { 1 }),
            stdout: stdout.into(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn workspace() -> (tempfile::TempDir, ProblemWorkspace) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("abc999");
        let problem = root.join("a");
        std::fs::create_dir_all(&problem).unwrap();
        std::fs::write(
            root.join("contest.json"),
            include_str!("../../tests/fixtures/json/contest.json"),
        )
        .unwrap();
        let mut template: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/json/template_legacy.json"
        ))
        .unwrap();
        template["pre_submit"] = serde_json::json!(["pre"]);
        std::fs::write(
            problem.join("template.json"),
            serde_json::to_vec(&template).unwrap(),
        )
        .unwrap();
        std::fs::write(problem.join("main.py"), "print(3)\n").unwrap();
        let workspace = ProblemWorkspace::discover_from(&problem).unwrap();
        (temp, workspace)
    }

    #[test]
    fn normalizes_line_endings_like_the_previous_reader() {
        assert_eq!(normalize_source_to_crlf("a\nb\n"), "a\r\nb\r\n");
        assert_eq!(normalize_source_to_crlf("a\r\nb"), "a\r\nb\r\n");
        assert_eq!(normalize_source_to_crlf(""), "");
    }

    #[tokio::test]
    async fn preparation_runs_pre_submit_then_tests_and_reads_source() {
        let (_temp, workspace) = workspace();
        let runner = FakeRunner::with_outputs([output(true, ""), output(true, "3\n")]);

        let source = prepare_solution(&workspace, &runner, false).await.unwrap();
        assert_eq!(source, "print(3)\r\n");
        assert_eq!(
            *runner.calls.lock().unwrap(),
            [
                vec!["pre".to_string()],
                vec!["python".into(), "main.py".into()]
            ]
        );
    }

    #[tokio::test]
    async fn preparation_short_circuits_on_pre_submit_or_sample_failure() {
        let (_temp, workspace) = workspace();
        let pre_failure = FakeRunner::with_outputs([output(false, "")]);
        assert!(
            prepare_solution(&workspace, &pre_failure, false)
                .await
                .is_err()
        );
        assert_eq!(pre_failure.calls.lock().unwrap().len(), 1);

        let sample_failure = FakeRunner::with_outputs([output(true, ""), output(true, "wrong")]);
        assert!(
            prepare_solution(&workspace, &sample_failure, false)
                .await
                .is_err()
        );
        assert_eq!(sample_failure.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn no_test_still_runs_pre_submit() {
        let (_temp, workspace) = workspace();
        let runner = FakeRunner::with_outputs([output(true, "")]);

        let source = prepare_solution(&workspace, &runner, true).await.unwrap();
        assert_eq!(source, "print(3)\r\n");
        assert_eq!(*runner.calls.lock().unwrap(), [vec!["pre".to_string()]]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_submit_file_symlinks_that_escape_the_problem_directory() {
        use std::os::unix::fs::symlink;

        let (temp, workspace) = workspace();
        let outside = temp.path().join("outside.py");
        std::fs::write(&outside, "secret\n").unwrap();
        std::fs::remove_file(workspace.submit_path()).unwrap();
        symlink(outside, workspace.submit_path()).unwrap();
        let runner = FakeRunner::with_outputs([output(true, "")]);

        assert!(prepare_solution(&workspace, &runner, true).await.is_err());
    }
}
