use crate::workspace::command::{CommandInput, CommandRunner};
use crate::workspace::problem::ProblemWorkspace;
use anyhow::Result;
use std::time::Duration;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum TestStatus {
    Ce,
    Re,
    Tle,
    Ole,
    Wa,
    Ac,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestResult {
    pub(crate) status: TestStatus,
    pub(crate) expected: String,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) exit_code: i32,
}

impl TestResult {
    pub(crate) fn is_failed(&self) -> bool {
        self.status != TestStatus::Ac
    }
}

pub(crate) async fn run_sample_tests(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
) -> Result<Vec<TestResult>> {
    let mut results = Vec::new();
    let config = workspace.template();

    if let Some(compile) = &config.compile_command {
        let output = runner
            .run(
                compile,
                workspace.problem_dir(),
                CommandInput::Inherit,
                COMPILE_TIMEOUT,
            )
            .await?;
        if !output.success {
            results.push(TestResult {
                status: TestStatus::Ce,
                expected: String::new(),
                stdout: output.stdout,
                stderr: stderr_with_timeout(output.stderr, output.timed_out, COMPILE_TIMEOUT),
                exit_code: output.exit_code.unwrap_or(-1),
            });
            return Ok(results);
        }
    }

    for sample in &workspace.problem().sample_cases {
        let timeout = Duration::from_millis(workspace.problem().time_limit_msecs as u64)
            .saturating_add(Duration::from_secs(2));
        let output = runner
            .run(
                &config.exec_command,
                workspace.problem_dir(),
                CommandInput::Bytes(sample.input.as_bytes().to_vec()),
                timeout,
            )
            .await?;
        let status = if output.timed_out {
            TestStatus::Tle
        } else if output.stdout_truncated || output.stderr_truncated {
            TestStatus::Ole
        } else if !output.success {
            TestStatus::Re
        } else if output.stdout.trim() != sample.expected.trim() {
            TestStatus::Wa
        } else {
            TestStatus::Ac
        };
        results.push(TestResult {
            status,
            expected: sample.expected.clone(),
            stdout: output.stdout,
            stderr: stderr_with_timeout(output.stderr, output.timed_out, timeout),
            exit_code: output.exit_code.unwrap_or(-1),
        });
    }

    Ok(results)
}

fn stderr_with_timeout(stderr: String, timed_out: bool, timeout: Duration) -> String {
    if !timed_out {
        return stderr;
    }
    let timeout_message = format!(
        "Command timed out after {:.3} seconds.",
        timeout.as_secs_f64()
    );
    if stderr.is_empty() {
        timeout_message
    } else {
        format!("{timeout_message}\n{stderr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::command::{CommandOutput, CommandSpec};
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct Call {
        command: Vec<String>,
        cwd: PathBuf,
        input: CommandInput,
        timeout: Duration,
    }

    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        calls: Mutex<Vec<Call>>,
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(
            &self,
            command: &CommandSpec,
            cwd: &Path,
            input: CommandInput,
            timeout: Duration,
        ) -> Result<CommandOutput> {
            self.calls.lock().unwrap().push(Call {
                command: command.words(),
                cwd: cwd.to_path_buf(),
                input,
                timeout,
            });
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

    fn workspace(compile_command: Option<&[&str]>) -> (tempfile::TempDir, ProblemWorkspace) {
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
        template["compile_command"] = compile_command
            .map(|words| serde_json::json!(words))
            .unwrap_or(serde_json::Value::Null);
        std::fs::write(
            problem.join("template.json"),
            serde_json::to_vec(&template).unwrap(),
        )
        .unwrap();
        let workspace = ProblemWorkspace::discover_from(&problem).unwrap();
        (temp, workspace)
    }

    #[test]
    fn adds_a_clear_timeout_diagnostic() {
        assert_eq!(
            stderr_with_timeout(String::new(), true, Duration::from_millis(2500)),
            "Command timed out after 2.500 seconds."
        );
        assert_eq!(
            stderr_with_timeout("partial error".into(), true, Duration::from_secs(1)),
            "Command timed out after 1.000 seconds.\npartial error"
        );
    }

    #[tokio::test]
    async fn classifies_sample_results_and_passes_execution_context() {
        for (mut command_output, expected_status) in [
            (output(true, "3\n"), TestStatus::Ac),
            (output(true, "4\n"), TestStatus::Wa),
            (output(false, ""), TestStatus::Re),
            (output(false, ""), TestStatus::Tle),
            (output(true, "3\n"), TestStatus::Ole),
        ] {
            if expected_status == TestStatus::Tle {
                command_output.timed_out = true;
            }
            if expected_status == TestStatus::Ole {
                command_output.stdout_truncated = true;
            }
            let (_temp, workspace) = workspace(None);
            let runner = FakeRunner::with_outputs([command_output]);

            let results = run_sample_tests(&workspace, &runner).await.unwrap();
            assert_eq!(results[0].status, expected_status);
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].command, ["python", "main.py"]);
            assert_eq!(calls[0].cwd, workspace.problem_dir());
            assert_eq!(calls[0].input, CommandInput::Bytes(b"1 2\n".to_vec()));
            assert_eq!(calls[0].timeout, Duration::from_secs(4));
        }
    }

    #[tokio::test]
    async fn compile_failure_becomes_ce_and_skips_samples() {
        let (_temp, workspace) = workspace(Some(&["compiler", "main.rs"]));
        let runner = FakeRunner::with_outputs([output(false, "compile output")]);

        let results = run_sample_tests(&workspace, &runner).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, TestStatus::Ce);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].command, ["compiler", "main.rs"]);
        assert_eq!(calls[0].input, CommandInput::Inherit);
        assert_eq!(calls[0].timeout, COMPILE_TIMEOUT);
    }
}
