use crate::workspace::command::{CommandInput, CommandOutput, CommandRunner, CommandSpec};
use crate::workspace::problem::ProblemWorkspace;
use anyhow::Result;
use std::time::Duration;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) fn execution_timeout(workspace: &ProblemWorkspace) -> Duration {
    Duration::from_millis(workspace.problem().time_limit_msecs as u64)
        .saturating_add(Duration::from_secs(2))
}

pub(crate) struct RunReport {
    pub(crate) compilation: Option<CompileResult>,
    pub(crate) execution: Option<CommandOutput>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CompileResult {
    pub(crate) output: CommandOutput,
    pub(crate) timeout: Duration,
}

pub(crate) async fn execute_command(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    command: &CommandSpec,
    input: CommandInput,
    timeout: Duration,
) -> Result<CommandOutput> {
    runner
        .run(command, workspace.problem_dir(), input, timeout)
        .await
}

pub(crate) async fn compile_program(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
) -> Result<Option<CompileResult>> {
    let Some(command) = &workspace.template().compile_command else {
        return Ok(None);
    };
    let output = execute_command(
        workspace,
        runner,
        command,
        CommandInput::Inherit,
        COMPILE_TIMEOUT,
    )
    .await?;
    Ok(Some(CompileResult {
        output,
        timeout: COMPILE_TIMEOUT,
    }))
}

pub(crate) async fn run_program(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    input: CommandInput,
) -> Result<RunReport> {
    let compilation = compile_program(workspace, runner).await?;
    if compilation
        .as_ref()
        .is_some_and(|result| !result.output.success)
    {
        return Ok(RunReport {
            compilation,
            execution: None,
        });
    }
    let execution = runner
        .run_passthrough(
            &workspace.template().exec_command,
            workspace.problem_dir(),
            input,
        )
        .await?;
    Ok(RunReport {
        compilation,
        execution: Some(execution),
    })
}

pub(crate) async fn execute_program(
    workspace: &ProblemWorkspace,
    runner: &dyn CommandRunner,
    input: CommandInput,
    timeout: Duration,
) -> Result<CommandOutput> {
    execute_command(
        workspace,
        runner,
        &workspace.template().exec_command,
        input,
        timeout,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    type Call = (Vec<String>, CommandInput, Option<Duration>, bool);

    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        calls: Mutex<Vec<Call>>,
    }

    #[async_trait]
    impl CommandRunner for FakeRunner {
        async fn run(
            &self,
            command: &CommandSpec,
            _cwd: &Path,
            input: CommandInput,
            timeout: Duration,
        ) -> Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((command.words(), input, Some(timeout), false));
            Ok(self.outputs.lock().unwrap().pop_front().unwrap())
        }

        async fn run_passthrough(
            &self,
            command: &CommandSpec,
            _cwd: &Path,
            input: CommandInput,
        ) -> Result<CommandOutput> {
            self.calls
                .lock()
                .unwrap()
                .push((command.words(), input, None, true));
            Ok(self.outputs.lock().unwrap().pop_front().unwrap())
        }
    }

    fn output(success: bool) -> CommandOutput {
        CommandOutput {
            success,
            timed_out: false,
            exit_code: Some(if success { 0 } else { 7 }),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            real_time: Duration::ZERO,
            cpu_user_time: None,
            cpu_system_time: None,
            peak_memory_bytes: None,
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
        template["compile_command"] = serde_json::json!(["compiler", "main.py"]);
        std::fs::write(
            problem.join("template.json"),
            serde_json::to_vec(&template).unwrap(),
        )
        .unwrap();
        let workspace = ProblemWorkspace::discover_from(&problem).unwrap();
        (temp, workspace)
    }

    #[tokio::test]
    async fn run_compiles_once_then_executes_with_selected_input() {
        let (_temp, workspace) = workspace();
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([output(true), output(false)])),
            calls: Mutex::new(Vec::new()),
        };
        let input = CommandInput::File(PathBuf::from("input.txt"));
        let report = run_program(&workspace, &runner, input.clone())
            .await
            .unwrap();
        assert_eq!(report.compilation.unwrap().output.exit_code, Some(0));
        assert_eq!(report.execution.unwrap().exit_code, Some(7));
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0],
            (
                vec!["compiler".into(), "main.py".into()],
                CommandInput::Inherit,
                Some(Duration::from_secs(120)),
                false
            )
        );
        assert_eq!(
            calls[1],
            (vec!["python".into(), "main.py".into()], input, None, true)
        );
    }

    #[tokio::test]
    async fn compile_failure_skips_execution() {
        let (_temp, workspace) = workspace();
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([output(false)])),
            calls: Mutex::new(Vec::new()),
        };
        let report = run_program(&workspace, &runner, CommandInput::Inherit)
            .await
            .unwrap();
        assert_eq!(report.compilation.unwrap().output.exit_code, Some(7));
        assert!(report.execution.is_none());
        assert_eq!(runner.calls.lock().unwrap().len(), 1);
    }
}
