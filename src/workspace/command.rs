use crate::workspace::process_tree::ProcessTree;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinSet;

const MAX_CAPTURE_BYTES: usize = 16 * 1024 * 1024;
const CAPTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandSpec {
    program: String,
    args: Vec<String>,
}

impl CommandSpec {
    pub(crate) fn from_words(words: Vec<String>) -> Result<Self> {
        let (program, args) = words.split_first().context("Command must not be empty.")?;
        if program.trim().is_empty() || program.contains('\0') {
            anyhow::bail!("Command program must not be empty.");
        }
        Ok(Self {
            program: program.clone(),
            args: args.to_vec(),
        })
    }

    pub(crate) fn words(&self) -> Vec<String> {
        std::iter::once(self.program.clone())
            .chain(self.args.iter().cloned())
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CommandInput {
    Inherit,
    Null,
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandOutput {
    pub(crate) success: bool,
    pub(crate) timed_out: bool,
    pub(crate) exit_code: Option<i32>,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) stdout_truncated: bool,
    pub(crate) stderr_truncated: bool,
    pub(crate) real_time: Duration,
    pub(crate) cpu_user_time: Option<Duration>,
    pub(crate) cpu_system_time: Option<Duration>,
    pub(crate) peak_memory_bytes: Option<u64>,
}

#[async_trait]
pub(crate) trait CommandRunner: Send + Sync {
    async fn run(
        &self,
        command: &CommandSpec,
        cwd: &Path,
        input: CommandInput,
        timeout: Duration,
    ) -> Result<CommandOutput>;
}

#[derive(Default)]
pub(crate) struct SystemCommandRunner;

#[async_trait]
impl CommandRunner for SystemCommandRunner {
    async fn run(
        &self,
        command: &CommandSpec,
        cwd: &Path,
        input: CommandInput,
        timeout: Duration,
    ) -> Result<CommandOutput> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        match input {
            CommandInput::Inherit => {
                process.stdin(Stdio::inherit());
            }
            CommandInput::Null => {
                process.stdin(Stdio::null());
            }
            CommandInput::Bytes(bytes) => {
                let mut input_file =
                    tempfile::tempfile().context("Failed to create temporary command input.")?;
                input_file
                    .write_all(&bytes)
                    .context("Failed to write temporary command input.")?;
                input_file
                    .seek(SeekFrom::Start(0))
                    .context("Failed to rewind temporary command input.")?;
                process.stdin(Stdio::from(input_file));
            }
        }
        let mut process_tree = ProcessTree::prepare(&mut process)?;

        let mut child = process
            .spawn()
            .with_context(|| format!("Failed to run command '{}'.", command.program))?;
        let started = Instant::now();
        if let Err(error) = process_tree.attach(&child) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("Failed to isolate command process tree.");
        }
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture command stdout.")?;
        let stderr = child
            .stderr
            .take()
            .context("Failed to capture command stderr.")?;
        let stdout = tokio::process::ChildStdout::from_std(stdout)
            .context("Failed to capture command stdout asynchronously.")?;
        let stderr = tokio::process::ChildStderr::from_std(stderr)
            .context("Failed to capture command stderr asynchronously.")?;
        let stdout_capture = Arc::new(Mutex::new(Capture::default()));
        let stderr_capture = Arc::new(Mutex::new(Capture::default()));
        let mut capture_tasks = JoinSet::new();
        capture_tasks.spawn(drain_capture(stdout, Arc::clone(&stdout_capture)));
        capture_tasks.spawn(drain_capture(stderr, Arc::clone(&stderr_capture)));

        let mut wait_task = tokio::task::spawn_blocking(move || wait_for_child(child));
        let (status, usage, timed_out) = match tokio::time::timeout(timeout, &mut wait_task).await {
            Ok(result) => {
                let (status, usage) = result.context("Failed to join command wait task.")??;
                (Some(status), usage, false)
            }
            Err(_) => {
                process_tree
                    .terminate()
                    .context("Failed to stop timed-out command process tree.")?;
                let (_, usage) = wait_task
                    .await
                    .context("Failed to join timed-out command wait task.")??;
                (None, usage, true)
            }
        };
        let real_time = started.elapsed();
        let usage = process_tree.resource_usage().or(usage);

        match tokio::time::timeout(
            CAPTURE_SHUTDOWN_TIMEOUT,
            finish_capture_tasks(&mut capture_tasks),
        )
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                capture_tasks.abort_all();
                finish_capture_tasks(&mut capture_tasks).await?;
            }
        }
        let stdout = finish_capture(&stdout_capture)?;
        let stderr = finish_capture(&stderr_capture)?;

        Ok(CommandOutput {
            success: status.as_ref().is_some_and(|status| status.success()),
            timed_out,
            exit_code: status.and_then(|status| status.code()),
            stdout: stdout.text,
            stderr: stderr.text,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
            real_time,
            cpu_user_time: usage.and_then(|usage| usage.user),
            cpu_system_time: usage.and_then(|usage| usage.system),
            peak_memory_bytes: usage.and_then(|usage| usage.peak_memory_bytes),
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct ResourceUsage {
    pub(super) user: Option<Duration>,
    pub(super) system: Option<Duration>,
    pub(super) peak_memory_bytes: Option<u64>,
}

#[cfg(unix)]
fn wait_for_child(child: std::process::Child) -> Result<(ExitStatus, Option<ResourceUsage>)> {
    use std::os::unix::process::ExitStatusExt;

    let pid = i32::try_from(child.id()).context("Command process ID is too large for wait4.")?;
    loop {
        let mut status = 0;
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        let result = unsafe { libc::wait4(pid, &mut status, 0, usage.as_mut_ptr()) };
        if result == pid {
            let usage = unsafe { usage.assume_init() };
            return Ok((
                ExitStatus::from_raw(status),
                Some(resource_usage_from_rusage(&usage)),
            ));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error).context("Failed to wait for command with wait4.");
        }
    }
}

#[cfg(unix)]
fn resource_usage_from_rusage(usage: &libc::rusage) -> ResourceUsage {
    fn timeval_duration(time: libc::timeval) -> Option<Duration> {
        let seconds = u64::try_from(time.tv_sec).ok()?;
        let micros = u32::try_from(time.tv_usec).ok()?;
        if micros >= 1_000_000 {
            return None;
        }
        Some(Duration::new(seconds, micros * 1_000))
    }

    #[cfg(target_os = "linux")]
    let peak_memory_bytes = u64::try_from(usage.ru_maxrss)
        .ok()
        .and_then(|kb| kb.checked_mul(1024));
    #[cfg(target_os = "macos")]
    let peak_memory_bytes = u64::try_from(usage.ru_maxrss).ok();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let peak_memory_bytes = None;

    ResourceUsage {
        user: timeval_duration(usage.ru_utime),
        system: timeval_duration(usage.ru_stime),
        peak_memory_bytes,
    }
}

#[cfg(windows)]
fn wait_for_child(mut child: std::process::Child) -> Result<(ExitStatus, Option<ResourceUsage>)> {
    Ok((child.wait().context("Failed to wait for command.")?, None))
}

async fn finish_capture_tasks(tasks: &mut JoinSet<Result<()>>) -> Result<()> {
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(result) => result?,
            Err(error) if error.is_cancelled() => {}
            Err(error) => return Err(error).context("Failed to join command capture task."),
        }
    }
    Ok(())
}

#[derive(Default)]
struct Capture {
    bytes: Vec<u8>,
    truncated: bool,
}

struct FinishedCapture {
    text: String,
    truncated: bool,
}

async fn drain_capture(
    mut reader: impl AsyncRead + Unpin,
    capture: Arc<Mutex<Capture>>,
) -> Result<()> {
    drain_capture_with_limit(&mut reader, capture, MAX_CAPTURE_BYTES).await
}

async fn drain_capture_with_limit(
    mut reader: impl AsyncRead + Unpin,
    capture: Arc<Mutex<Capture>>,
    limit: usize,
) -> Result<()> {
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut chunk)
            .await
            .context("Failed to read command output.")?;
        if read == 0 {
            return Ok(());
        }
        let mut capture = capture.lock().expect("capture mutex poisoned");
        let remaining = limit.saturating_sub(capture.bytes.len());
        let retained = remaining.min(read);
        capture.bytes.extend_from_slice(&chunk[..retained]);
        capture.truncated |= retained < read;
    }
}

fn finish_capture(capture: &Mutex<Capture>) -> Result<FinishedCapture> {
    let capture = capture
        .lock()
        .map_err(|_| anyhow::anyhow!("Failed to capture command output."))?;
    let mut output = String::from_utf8_lossy(&capture.bytes).into_owned();
    if capture.truncated {
        output.push_str("\n[output truncated by ackit]\n");
    }
    Ok(FinishedCapture {
        text: output,
        truncated: capture.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn command_spec_requires_a_program() {
        assert!(CommandSpec::from_words(Vec::new()).is_err());
        assert!(CommandSpec::from_words(vec![String::new()]).is_err());
        let command = CommandSpec::from_words(vec!["python".into(), "main.py".into()]).unwrap();
        assert_eq!(command.words(), ["python", "main.py"]);
    }

    #[tokio::test]
    async fn capture_retains_only_the_configured_limit() {
        let capture = Arc::new(Mutex::new(Capture::default()));
        drain_capture_with_limit(&b"abcdef"[..], Arc::clone(&capture), 3)
            .await
            .unwrap();
        let finished = finish_capture(&capture).unwrap();
        assert!(finished.truncated);
        assert_eq!(finished.text, "abc\n[output truncated by ackit]\n");
    }

    #[tokio::test]
    async fn system_runner_passes_stdin_and_cwd_without_a_shell() {
        let temp = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let command = CommandSpec::from_words(vec![
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::command_helper".into(),
            "--nocapture".into(),
        ])
        .unwrap();
        let output = SystemCommandRunner
            .run(
                &command,
                temp.path(),
                CommandInput::Bytes(b"sample input".to_vec()),
                Duration::from_secs(5),
            )
            .await
            .unwrap();

        assert!(output.success, "{}", output.stderr);
        assert!(output.stdout.contains("sample input"));
        assert!(output.stdout.contains(&temp.path().display().to_string()));
        assert!(output.real_time > Duration::ZERO);
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            assert!(output.cpu_user_time.is_some());
            assert!(output.cpu_system_time.is_some());
            assert!(output.peak_memory_bytes.is_some_and(|bytes| bytes > 0));
        }
    }

    #[tokio::test]
    async fn system_runner_stops_a_timed_out_process_tree() {
        let temp = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let command = CommandSpec::from_words(vec![
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::timeout_helper".into(),
            "--nocapture".into(),
        ])
        .unwrap();
        let output = SystemCommandRunner
            .run(
                &command,
                temp.path(),
                CommandInput::Null,
                Duration::from_millis(50),
            )
            .await
            .unwrap();

        assert!(output.timed_out);
        assert!(!output.success);
        assert_eq!(output.exit_code, None);
        assert!(output.real_time >= Duration::from_millis(50));
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        {
            assert!(output.cpu_user_time.is_some());
            assert!(output.cpu_system_time.is_some());
            assert!(output.peak_memory_bytes.is_some_and(|bytes| bytes > 0));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            !temp.path().join("descendant-alive").exists(),
            "a descendant survived after the command timed out"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_command_stays_suspended_until_job_attachment() {
        let temp = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let mut process = Command::new(executable);
        process
            .args([
                "--ignored",
                "--exact",
                "workspace::command::tests::windows_start_helper",
                "--nocapture",
            ])
            .current_dir(temp.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut process_tree = ProcessTree::prepare(&mut process).unwrap();
        let mut child = process.spawn().unwrap();

        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(
            !temp.path().join("command-started").exists(),
            "the command ran before it was attached to the Job Object"
        );

        process_tree.attach(&child).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(temp.path().join("command-started").exists());
    }

    #[test]
    #[ignore]
    fn command_helper() {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        println!("cwd={}", std::env::current_dir().unwrap().display());
        println!("stdin={input}");
    }

    #[test]
    #[ignore]
    fn timeout_helper() {
        let mut descendant = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "workspace::command::tests::descendant_helper",
                "--nocapture",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_secs(10));
        descendant.wait().unwrap();
    }

    #[test]
    #[ignore]
    fn descendant_helper() {
        std::thread::sleep(Duration::from_millis(250));
        std::fs::write("descendant-alive", b"survived").unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore]
    fn windows_start_helper() {
        std::fs::write("command-started", b"started").unwrap();
    }
}
