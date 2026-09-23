use crate::workspace::command::{CommandInput, CommandOutput, CommandRunner, CommandSpec};
use crate::workspace::problem::ProblemWorkspace;
use anyhow::Result;
use std::time::Duration;

const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

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
