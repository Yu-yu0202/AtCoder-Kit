use crate::application::program::{CompileResult, compile_program, execute_program};
use crate::workspace::command::{CommandInput, CommandOutput, CommandRunner};
use crate::workspace::problem::ProblemWorkspace;
use anyhow::{Result, bail};
use std::num::NonZeroUsize;
use std::time::Duration;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) enum SampleCaseStatus {
    Ac,
    Wa,
    Re,
    Tle,
    Ole,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SampleCaseResult {
    pub(crate) index: NonZeroUsize,
    pub(crate) status: SampleCaseStatus,
    pub(crate) expected: String,
    pub(crate) output: CommandOutput,
    pub(crate) timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SampleTestReport {
    pub(crate) compilation: Option<CompileResult>,
    pub(crate) cases: Vec<SampleCaseResult>,
}

impl SampleTestReport {
    pub(crate) fn compile_failed(&self) -> bool {
        self.compilation
            .as_ref()
            .is_some_and(|compilation| !compilation.output.success)
    }

    pub(crate) fn is_success(&self) -> bool {
        !self.compile_failed()
            && self
                .cases
                .iter()
                .all(|case| case.status == SampleCaseStatus::Ac)
    }
}

pub(crate) async fn run_sample_tests(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
) -> Result<SampleTestReport> {
    run_sample_tests_impl(workspace, runner, None).await
}

pub(crate) async fn run_sample_tests_selected(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    selected_case: Option<NonZeroUsize>,
) -> Result<SampleTestReport> {
    let samples = &workspace.problem().sample_cases;
    if samples.is_empty() {
        bail!("No sample cases are available.");
    }
    if let Some(index) = selected_case
        && index.get() > samples.len()
    {
        bail!(
            "Sample case {} does not exist ({} available).",
            index,
            samples.len()
        );
    }
    run_sample_tests_impl(workspace, runner, selected_case).await
}

async fn run_sample_tests_impl(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    selected_case: Option<NonZeroUsize>,
) -> Result<SampleTestReport> {
    let samples = &workspace.problem().sample_cases;
    let compilation = compile_program(workspace, runner).await?;
    if compilation
        .as_ref()
        .is_some_and(|compilation| !compilation.output.success)
    {
        return Ok(SampleTestReport {
            compilation,
            cases: Vec::new(),
        });
    }

    let mut cases = Vec::new();
    for (position, sample) in samples.iter().enumerate() {
        let index = NonZeroUsize::new(position + 1).expect("sample case index must be non-zero");
        if selected_case.is_some_and(|selected| selected != index) {
            continue;
        }
        let timeout = Duration::from_millis(workspace.problem().time_limit_msecs as u64)
            .saturating_add(Duration::from_secs(2));
        let output = execute_program(
            workspace,
            runner,
            CommandInput::Bytes(sample.input.as_bytes().to_vec()),
            timeout,
        )
        .await?;
        let status = if output.timed_out {
            SampleCaseStatus::Tle
        } else if output.stdout_truncated || output.stderr_truncated {
            SampleCaseStatus::Ole
        } else if !output.success {
            SampleCaseStatus::Re
        } else if output.stdout.trim() != sample.expected.trim() {
            SampleCaseStatus::Wa
        } else {
            SampleCaseStatus::Ac
        };
        cases.push(SampleCaseResult {
            index,
            status,
            expected: sample.expected.clone(),
            output,
            timeout,
        });
    }

    Ok(SampleTestReport { compilation, cases })
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
            real_time: Duration::ZERO,
            cpu_user_time: None,
            cpu_system_time: None,
            peak_memory_bytes: None,
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

    #[tokio::test]
    async fn classifies_sample_results_and_passes_execution_context() {
        for (mut command_output, expected_status) in [
            (output(true, "3\n"), SampleCaseStatus::Ac),
            (output(true, "4\n"), SampleCaseStatus::Wa),
            (output(false, ""), SampleCaseStatus::Re),
            (output(false, ""), SampleCaseStatus::Tle),
            (output(true, "3\n"), SampleCaseStatus::Ole),
        ] {
            if expected_status == SampleCaseStatus::Tle {
                command_output.timed_out = true;
            }
            if expected_status == SampleCaseStatus::Ole {
                command_output.stdout_truncated = true;
            }
            let (_temp, workspace) = workspace(None);
            let runner = FakeRunner::with_outputs([command_output]);

            let report = run_sample_tests(&workspace, &runner).await.unwrap();
            assert!(report.compilation.is_none());
            assert_eq!(report.cases.len(), 1);
            assert_eq!(report.cases[0].index, NonZeroUsize::new(1).unwrap());
            assert_eq!(report.cases[0].status, expected_status);
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
        let mut compile_output = output(false, "compile output");
        compile_output.stderr = "raw compiler stderr".into();
        compile_output.exit_code = None;
        let runner = FakeRunner::with_outputs([compile_output.clone()]);

        let report = run_sample_tests(&workspace, &runner).await.unwrap();
        assert!(report.compile_failed());
        assert!(!report.is_success());
        assert!(report.cases.is_empty());
        let compilation = report.compilation.as_ref().unwrap();
        assert_eq!(compilation.timeout, Duration::from_secs(120));
        assert_eq!(compilation.output, compile_output);
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].command, ["compiler", "main.rs"]);
        assert_eq!(calls[0].input, CommandInput::Inherit);
        assert_eq!(calls[0].timeout, Duration::from_secs(120));
    }

    #[tokio::test]
    async fn retains_raw_case_output_and_optional_exit_code() {
        let (_temp, workspace) = workspace(None);
        let mut command_output = output(true, "3\n");
        command_output.stderr = "raw stderr\n".into();
        command_output.exit_code = None;
        let runner = FakeRunner::with_outputs([command_output.clone()]);

        let report = run_sample_tests(&workspace, &runner).await.unwrap();
        let case = &report.cases[0];
        assert_eq!(case.output, command_output);
        assert_eq!(case.timeout, Duration::from_secs(4));
        assert!(report.is_success());
    }

    #[test]
    fn success_requires_compilation_and_case_success() {
        let empty = SampleTestReport {
            compilation: None,
            cases: Vec::new(),
        };
        assert!(!empty.compile_failed());
        assert!(empty.is_success());

        let compiled = SampleTestReport {
            compilation: Some(CompileResult {
                output: output(true, ""),
                timeout: Duration::from_secs(120),
            }),
            cases: Vec::new(),
        };
        assert!(!compiled.compile_failed());
        assert!(compiled.is_success());

        let failed_case = SampleCaseResult {
            index: NonZeroUsize::new(1).unwrap(),
            status: SampleCaseStatus::Wa,
            expected: String::new(),
            output: output(true, ""),
            timeout: Duration::from_secs(4),
        };
        let failed_cases = SampleTestReport {
            compilation: None,
            cases: vec![failed_case],
        };
        assert!(!failed_cases.is_success());
    }
    #[tokio::test]
    async fn selects_original_case_number_and_rejects_missing_case_before_compile() {
        let (temp, _) = workspace(Some(&["compiler", "main.rs"]));
        let contest_path = temp.path().join("abc999/contest.json");
        let mut contest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&contest_path).unwrap()).unwrap();
        contest["problems"]["A"]["sample_cases"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"input": "4 5\n", "expected": "9\n"}));
        std::fs::write(&contest_path, serde_json::to_vec(&contest).unwrap()).unwrap();
        let workspace = ProblemWorkspace::discover_from(&temp.path().join("abc999/a")).unwrap();
        let runner = FakeRunner::with_outputs([output(true, ""), output(true, "9\n")]);
        let report = run_sample_tests_selected(&workspace, &runner, NonZeroUsize::new(2))
            .await
            .unwrap();
        assert_eq!(report.cases.len(), 1);
        assert_eq!(report.cases[0].index.get(), 2);
        {
            let calls = runner.calls.lock().unwrap();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[1].input, CommandInput::Bytes(b"4 5\n".to_vec()));
        }

        let empty_runner = FakeRunner::with_outputs([]);
        let error = run_sample_tests_selected(&workspace, &empty_runner, NonZeroUsize::new(3))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Sample case 3 does not exist"));
        assert!(empty_runner.calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_samples_reject_cli_test_without_changing_submit_path() {
        let (temp, _) = workspace(None);
        let contest_path = temp.path().join("abc999/contest.json");
        let mut contest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&contest_path).unwrap()).unwrap();
        contest["problems"]["A"]["sample_cases"] = serde_json::json!([]);
        std::fs::write(&contest_path, serde_json::to_vec(&contest).unwrap()).unwrap();
        let workspace = ProblemWorkspace::discover_from(&temp.path().join("abc999/a")).unwrap();
        let runner = FakeRunner::with_outputs([]);
        assert!(
            run_sample_tests_selected(&workspace, &runner, None)
                .await
                .unwrap_err()
                .to_string()
                .contains("No sample cases")
        );
        assert!(runner.calls.lock().unwrap().is_empty());
        let report = run_sample_tests(&workspace, &runner).await.unwrap();
        assert!(report.is_success());
        assert!(report.cases.is_empty());
    }
}
