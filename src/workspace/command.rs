use crate::workspace::process_tree::ProcessTree;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
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
    File(PathBuf),
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

    async fn run_passthrough(
        &self,
        command: &CommandSpec,
        cwd: &Path,
        input: CommandInput,
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
        self.run_with_output(command, cwd, input, Some(timeout), false)
            .await
    }

    async fn run_passthrough(
        &self,
        command: &CommandSpec,
        cwd: &Path,
        input: CommandInput,
    ) -> Result<CommandOutput> {
        self.run_with_output(command, cwd, input, None, true).await
    }
}

impl SystemCommandRunner {
    async fn run_with_output(
        &self,
        command: &CommandSpec,
        cwd: &Path,
        input: CommandInput,
        timeout: Option<Duration>,
        passthrough: bool,
    ) -> Result<CommandOutput> {
        let mut process = Command::new(&command.program);
        process.args(&command.args).current_dir(cwd);
        if passthrough {
            process.stdout(Stdio::inherit()).stderr(Stdio::inherit());
        } else {
            process.stdout(Stdio::piped()).stderr(Stdio::piped());
        }

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
            CommandInput::File(path) => {
                let input_file = std::fs::File::open(&path)
                    .with_context(|| format!("Failed to open input file '{}'.", path.display()))?;
                process.stdin(Stdio::from(input_file));
            }
        }
        #[cfg(unix)]
        let mut interrupt_signal = if passthrough {
            Some(
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                    .context("Failed to listen for Ctrl+C.")?,
            )
        } else {
            None
        };
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
        #[cfg(unix)]
        if passthrough && let Err(error) = process_tree.make_foreground() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error).context("Failed to give terminal to command.");
        }
        let stdout_capture = Arc::new(Mutex::new(Capture::default()));
        let stderr_capture = Arc::new(Mutex::new(Capture::default()));
        let mut capture_tasks = JoinSet::new();
        if !passthrough {
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
            capture_tasks.spawn(drain_capture(stdout, Arc::clone(&stdout_capture)));
            capture_tasks.spawn(drain_capture(stderr, Arc::clone(&stderr_capture)));
        }

        let mut wait_task = tokio::task::spawn_blocking(move || wait_for_child(child));
        let wait_result = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, &mut wait_task).await.ok(),
            None => {
                #[cfg(unix)]
                {
                    tokio::select! {
                        result = &mut wait_task => Some(result),
                        _ = interrupt_signal.as_mut().expect("passthrough has SIGINT listener").recv() => {
                            process_tree.terminate().context("Failed to stop interrupted command process tree.")?;
                            wait_task.await.context("Failed to join interrupted command wait task.")??;
                            anyhow::bail!("Program interrupted by Ctrl+C.");
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    Some((&mut wait_task).await)
                }
            }
        };
        let (status, usage, timed_out) = match wait_result {
            Some(result) => {
                let (status, usage) = result.context("Failed to join command wait task.")??;
                (Some(status), usage, false)
            }
            None => {
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

    #[cfg(unix)]
    #[test]
    fn sigint_reaps_redirected_passthrough_child() {
        let temp = tempfile::tempdir().unwrap();
        let pid_file = temp.path().join("child.pid");
        let mut parent = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "workspace::command::tests::interrupt_parent_helper",
                "--nocapture",
            ])
            .env("ACKIT_INTERRUPT_PID_FILE", &pid_file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let child_pid: i32 = loop {
            if let Ok(pid) = std::fs::read_to_string(&pid_file) {
                break pid.parse().unwrap();
            }
            assert!(Instant::now() < deadline, "child did not start");
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_eq!(unsafe { libc::kill(parent.id() as i32, libc::SIGINT) }, 0);
        let status = loop {
            if let Some(status) = parent.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                parent.kill().unwrap();
                parent.wait().unwrap();
                panic!("runner did not exit after SIGINT");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "runner failed: {status}");
        assert_eq!(unsafe { libc::kill(child_pid, 0) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn interrupt_parent_helper() {
        let command = CommandSpec::from_words(vec![
            std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::interrupt_child_helper".into(),
            "--nocapture".into(),
        ])
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(SystemCommandRunner.run_passthrough(
                &command,
                &std::env::current_dir().unwrap(),
                CommandInput::Null,
            ))
            .unwrap_err();
        assert_eq!(error.to_string(), "Program interrupted by Ctrl+C.");
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn interrupt_child_helper() {
        let pid_file = std::env::var_os("ACKIT_INTERRUPT_PID_FILE").unwrap();
        std::fs::write(pid_file, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(10));
    }

    #[tokio::test]
    async fn passthrough_waits_beyond_a_sample_time_limit() {
        let executable = std::env::current_exe().unwrap();
        let command = CommandSpec::from_words(vec![
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::slow_success_helper".into(),
            "--nocapture".into(),
        ])
        .unwrap();
        let output = SystemCommandRunner
            .run_passthrough(
                &command,
                &std::env::current_dir().unwrap(),
                CommandInput::Null,
            )
            .await
            .unwrap();
        assert!(output.success);
        assert!(!output.timed_out);
        assert!(output.real_time >= Duration::from_millis(150));
    }

    #[test]
    #[ignore]
    fn slow_success_helper() {
        std::thread::sleep(Duration::from_millis(200));
    }

    #[tokio::test]
    async fn passthrough_uses_file_stdin_without_capturing_output() {
        let temp = tempfile::tempdir().unwrap();
        let input_path = temp.path().join("input.txt");
        std::fs::write(&input_path, b"file input").unwrap();
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
            .run_passthrough(&command, temp.path(), CommandInput::File(input_path))
            .await
            .unwrap();
        assert!(output.success);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        assert!(!output.stdout_truncated);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn interactive_stdin_returns_terminal_to_invoking_shell() {
        let executable = std::env::current_exe().unwrap();
        let nested = shell_words::join([
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::pty_input_helper".into(),
            "--nocapture".into(),
        ]);
        let shell = format!("{nested}; read after; printf 'shell=%s\\n' \"$after\"");
        let mut script = match Command::new("timeout")
            .args([
                "-k",
                "1s",
                "8s",
                "script",
                "-q",
                "-e",
                "-c",
                &shell,
                "/dev/null",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(script) => script,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("Failed to start PTY test: {error}"),
        };
        script
            .stdin
            .take()
            .unwrap()
            .write_all(b"first\nsecond\n")
            .unwrap();
        let output = script.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "stdout: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("stdin=first"), "{stdout}");
        assert!(stdout.contains("shell=second"), "{stdout}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn file_input_with_tostop_returns_terminal_to_invoking_shell() {
        let executable = std::env::current_exe().unwrap();
        let nested = shell_words::join([
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::pty_input_helper".into(),
            "--nocapture".into(),
        ]);
        let shell = format!(
            "stty tostop; ACKIT_PTY_FILE_INPUT=1 {nested}; read after; printf 'shell=%s\\n' \"$after\""
        );
        let mut script = match Command::new("timeout")
            .args([
                "-k",
                "1s",
                "8s",
                "script",
                "-q",
                "-e",
                "-c",
                &shell,
                "/dev/null",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(script) => script,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("Failed to start PTY test: {error}"),
        };
        script.stdin.take().unwrap().write_all(b"second\n").unwrap();
        let output = script.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "stdout: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("stdin=file input"), "{stdout}");
        assert!(stdout.contains("shell=second"), "{stdout}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn timed_out_command_returns_terminal_to_invoking_shell() {
        let executable = std::env::current_exe().unwrap();
        let nested = shell_words::join([
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            "workspace::command::tests::pty_input_helper".into(),
            "--nocapture".into(),
        ]);
        let shell =
            format!("ACKIT_PTY_TIMEOUT=1 {nested}; read after; printf 'shell=%s\\n' \"$after\"");
        let mut script = match Command::new("timeout")
            .args([
                "-k",
                "1s",
                "8s",
                "script",
                "-q",
                "-e",
                "-c",
                &shell,
                "/dev/null",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(script) => script,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("Failed to start PTY test: {error}"),
        };
        script.stdin.take().unwrap().write_all(b"after\n").unwrap();
        let output = script.wait_with_output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "stdout: {stdout}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains("shell=after"), "{stdout}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn pty_input_helper() {
        let executable = std::env::current_exe().unwrap();
        let timed_out = std::env::var_os("ACKIT_PTY_TIMEOUT").is_some();
        let helper = if timed_out {
            "workspace::command::tests::pty_slow_helper"
        } else {
            "workspace::command::tests::pty_reader_helper"
        };
        let command = CommandSpec::from_words(vec![
            executable.to_string_lossy().into_owned(),
            "--ignored".into(),
            "--exact".into(),
            helper.into(),
            "--nocapture".into(),
        ])
        .unwrap();
        let file = if std::env::var_os("ACKIT_PTY_FILE_INPUT").is_some() {
            let file = tempfile::NamedTempFile::new().unwrap();
            std::fs::write(file.path(), b"file input\n").unwrap();
            Some(file)
        } else {
            None
        };
        let input = file
            .as_ref()
            .map(|file| CommandInput::File(file.path().to_path_buf()))
            .unwrap_or(CommandInput::Inherit);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let timeout = timed_out.then_some(Duration::from_millis(100));
        let output = runtime
            .block_on(SystemCommandRunner.run_with_output(
                &command,
                &std::env::current_dir().unwrap(),
                input,
                timeout,
                true,
            ))
            .unwrap();
        assert_eq!(output.timed_out, timed_out);
        if !timed_out {
            assert!(output.success, "child exit: {:?}", output.exit_code);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn pty_slow_helper() {
        std::thread::sleep(Duration::from_secs(10));
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore]
    fn pty_reader_helper() {
        use std::io::BufRead;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).unwrap();
        println!("stdin={}", line.trim_end());
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
