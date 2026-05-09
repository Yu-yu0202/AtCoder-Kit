use crate::client::contest::from_file as get_contest;
use crate::core::template::from_file as get_template;
use anyhow::*;
use std::env;
use std::io::{Seek, SeekFrom, Write};
use std::process::Stdio;
use tempfile::tempfile;
use tokio::process;

#[derive(Eq, PartialEq, Debug)]
pub enum TestStatus {
    CE,
    RE,
    WA,
    AC,
}

#[derive(Debug)]
pub struct TestResult {
    pub status: TestStatus,
    pub expected: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

impl TestResult {
    pub fn is_ac(&self) -> bool {
        self.status == TestStatus::AC
    }

    pub fn is_failed(&self) -> bool {
        !self.is_ac()
    }
}

pub async fn test() -> Result<Vec<TestResult>> {
    let mut results: Vec<TestResult> = Vec::new();

    let config = get_template()?;
    let compile_command = &config.compile_command;
    let exec_command = &config.exec_command;
    let contest = get_contest()?;
    let problem_name = env::current_dir()
        .context("Failed to get current directory.")?
        .file_name()
        .context("Failed to get current directory name.")?
        .to_string_lossy()
        .to_uppercase();
    let problem = contest.problems.get(&problem_name).context(format!(
        "Failed to get problem {} from contest.",
        problem_name
    ))?;

    if let Some(compile_command) = compile_command {
        let cmd = process::Command::new(&compile_command[0])
            .args(&compile_command[1..])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to run compile command.")?;

        let result = cmd
            .wait_with_output()
            .await
            .context("Failed to get compile command result.")?;

        let stderr = String::from_utf8_lossy(&result.stderr);
        let stdout = String::from_utf8_lossy(&result.stdout);

        if !result.status.success() {
            results.push(TestResult {
                status: TestStatus::CE,
                expected: "".to_string(),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
                exit_code: result.status.code().unwrap_or(-1),
            });
            return Ok(results);
        }
    }

    if exec_command.is_empty() {
        bail!("Exec command is empty.");
    }

    for case in &problem.sample_cases {
        let mut temp_file = tempfile().context("Failed to create temporary file.")?;

        temp_file
            .write_all(case.input.as_bytes())
            .context("Failed to write to temporary file.")?;

        temp_file
            .seek(SeekFrom::Start(0))
            .context("Failed to seek temporary file.")?;

        let mut cmd = process::Command::new(&exec_command[0]);

        if exec_command.len() > 1 {
            cmd.args(&exec_command[1..]);
        }

        let cmd = cmd
            .stdin(Stdio::from(temp_file))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to run test.")?;

        let result = cmd
            .wait_with_output()
            .await
            .context("Failed to get test result.")?;

        let stdout = String::from_utf8_lossy(&result.stdout);
        let stderr = String::from_utf8_lossy(&result.stderr);

        if !result.status.success() {
            results.push(TestResult {
                status: TestStatus::RE,
                expected: case.expected.clone(),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
                exit_code: result.status.code().unwrap_or(-1),
            });
            continue;
        }

        if stdout.trim() != case.expected.trim() {
            results.push(TestResult {
                status: TestStatus::WA,
                expected: case.expected.clone(),
                stdout: stdout.to_string(),
                stderr: stderr.to_string(),
                exit_code: result.status.code().unwrap_or(-1),
            });
            continue;
        }

        results.push(TestResult {
            status: TestStatus::AC,
            expected: case.expected.clone(),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code: result.status.code().unwrap_or(-1),
        });
    }

    Ok(results)
}
