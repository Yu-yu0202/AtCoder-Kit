use crate::application::sample::{SampleCaseStatus, SampleTestReport, TimingBasis};
use crate::application::{
    AppEvent, Application, LoginOutcome, RunReport, SessionStatus, TemplateDetails, TemplateSummary,
};
use crate::workspace::command::CommandOutput;
use crate::workspace::template::NewTemplate;
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use log::{info, warn};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "ackit", version, about = format!("{}", "AtCoder-Kit".green().bold()))]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug, Eq, PartialEq)]
enum Commands {
    /// Login to atcoder.jp
    Login {
        /// Overwrite the existing REVEL_SESSION cookie value
        #[arg(long)]
        overwrite: bool,
    },
    /// Logout from atcoder.jp
    Logout,
    /// Show login status / username
    Whoami,
    /// Download contest problems
    #[command(visible_aliases = ["d", "n"])]
    Download {
        /// Contest ID (ex. abc001, ahc001, awc0001)
        contest_id: String,
        /// Template name
        #[arg(short = 't', long = "template" , conflicts_with = "no_template")]
        template_name: Option<String>,
        /// Skip clone default template
        #[arg(short, long)]
        no_template: bool,
    },
    /// Test the program with sample cases
    #[command(visible_alias = "t")]
    Test {
        /// Run only the specified sample case (1-based)
        #[arg(long = "case")]
        case: Option<NonZeroUsize>,
        /// Show detailed process resource metrics
        #[arg(long)]
        metrics: bool,
    },
    /// Compile and run the current solution
    Run {
        /// Read program stdin from this file
        #[arg(long)]
        input: Option<PathBuf>,
    },
    /// Submit the program
    #[command(visible_alias = "s")]
    Submit {
        /// Skip test before submission
        #[arg(short, long)]
        no_test: bool,
    },
    Template {
        #[command(subcommand)]
        action: TemplateCommand,
    },
}

#[derive(Subcommand, Debug, Eq, PartialEq)]
enum TemplateCommand {
    /// List registered templates
    List,
    /// Show a registered template
    Show {
        /// Template name
        name: String,
    },
    /// Set the default template
    SetDefault {
        /// Template name
        name: String,
    },
    /// Create new template files
    New {
        /// Template name
        name: String,
        /// The source code file (ex. main.py, main.cpp, src/main.rs)
        submit_file: String,
        /// The executable file or command to run the program
        /// (ex. "python3 main.py", a.out, target/debug/a)
        exec_command: String,
        /// Compiler command for compiled languages
        /// (ex. "g++ main.cpp -O3", "cargo build")
        #[arg(short, long)]
        compile_command: Option<String>,
        /// Pre-submit command (ex. execute a bundler)
        #[arg(short, long)]
        pre_submit: Option<String>,
        /// Set as the default template
        #[arg(short, long)]
        default: bool,
    },
}

fn show_templates(templates: &[TemplateSummary]) {
    if templates.is_empty() {
        info!("No templates found.");
        return;
    }

    info!("Templates:");
    for template in templates {
        let default = if template.is_default {
            " (default)"
        } else {
            ""
        };
        info!("  {}{default}", template.name);
    }
}

fn show_template(template: &TemplateDetails) {
    info!("Template: {}", template.name);
    info!(
        "Default: {}",
        if template.is_default { "yes" } else { "no" }
    );
    info!("Directory: {}", template.path.display());
    info!("Submit file: {}", template.submit_file.display());
    info!("Language ID: {}", template.language_id);
    info!(
        "Exec command: {}",
        shell_words::join(&template.exec_command)
    );
    info!(
        "Compile command: {}",
        template
            .compile_command
            .as_ref()
            .map(shell_words::join)
            .unwrap_or_else(|| "-".into())
    );
    info!(
        "Pre-submit command: {}",
        template
            .pre_submit
            .as_ref()
            .map(shell_words::join)
            .unwrap_or_else(|| "-".into())
    );
}

pub(crate) fn parse() -> Cli {
    Cli::parse()
}

fn show_event(event: AppEvent) {
    match event {
        AppEvent::FetchingContest(id) => info!("Fetching contest '{id}'..."),
        AppEvent::SavingContest(id) => info!("Saving contest to '{id}'..."),
        AppEvent::Testing => info!("Testing..."),
        AppEvent::TestSuccessful => info!("Test successful."),
        AppEvent::Submitting => info!("Submitting..."),
        AppEvent::SubmitSuccessful => info!("Submit successful."),
        AppEvent::LoggingIn => info!("Logging in..."),
        AppEvent::LoggingOut => info!("Logging out..."),
    }
}

fn stderr_with_timeout(output: &CommandOutput, timeout: Duration) -> String {
    if !output.timed_out {
        return output.stderr.clone();
    }
    let timeout_message = format!(
        "Command timed out after {:.3} seconds.",
        timeout.as_secs_f64()
    );
    if output.stderr.is_empty() {
        timeout_message
    } else {
        format!("{timeout_message}\n{}", output.stderr)
    }
}

fn show_test_results(report: &SampleTestReport, metrics: bool) {
    if let Some(compilation) = report.compilation.as_ref()
        && !compilation.output.success
    {
        warn!("{}", "Compile Error".red().bold());
        info!(
            "compiler exit code: {}",
            compilation.output.exit_code.unwrap_or(-1)
        );
        info!(
            "stderr:\n{}",
            stderr_with_timeout(&compilation.output, compilation.timeout)
        );
        info!("stdout:\n{}", compilation.output.stdout);
    }

    for result in &report.cases {
        info!(
            "Case {}: {:.3} s",
            result.index,
            report.case_time(result).as_secs_f64()
        );
        if metrics {
            info!(
                "  real: {:.3} s, CPU user: {}, CPU system: {}, peak memory: {}",
                result.output.real_time.as_secs_f64(),
                result
                    .output
                    .cpu_user_time
                    .map(|time| format!("{:.3} s", time.as_secs_f64()))
                    .unwrap_or_else(|| "unavailable".into()),
                result
                    .output
                    .cpu_system_time
                    .map(|time| format!("{:.3} s", time.as_secs_f64()))
                    .unwrap_or_else(|| "unavailable".into()),
                result
                    .output
                    .peak_memory_bytes
                    .map(|bytes| format!("{bytes} bytes"))
                    .unwrap_or_else(|| "unavailable".into())
            );
        }
        match result.status {
            SampleCaseStatus::Ac => info!("{}", "AC".green().bold()),
            SampleCaseStatus::Wa => {
                warn!("{}", "Wrong Answer".red().bold());
                info!("expected:\n{}", result.expected);
                info!("got:\n{}", result.output.stdout);
            }
            SampleCaseStatus::Re => {
                warn!("{}", "Runtime Error".red().bold());
                info!("exit code: {}", result.output.exit_code.unwrap_or(-1));
                info!(
                    "stderr:\n{}",
                    stderr_with_timeout(&result.output, result.timeout)
                );
            }
            SampleCaseStatus::Tle => {
                warn!("{}", "Time Limit Exceeded".red().bold());
                info!(
                    "stderr:\n{}",
                    stderr_with_timeout(&result.output, result.timeout)
                );
            }
            SampleCaseStatus::Ole => {
                warn!("{}", "Output Limit Exceeded".red().bold());
                info!("stdout:\n{}", result.output.stdout);
                info!(
                    "stderr:\n{}",
                    stderr_with_timeout(&result.output, result.timeout)
                );
            }
        }
    }
    if let Some(summary) = report.timing_summary() {
        let basis = match summary.basis {
            TimingBasis::CpuOrReal => "max(CPU, real)",
            TimingBasis::RealOnly => "real only",
        };
        info!(
            "Time ({basis}, min/avg/max): {:.3} / {:.3} / {:.3} s",
            summary.min.as_secs_f64(),
            summary.avg.as_secs_f64(),
            summary.max.as_secs_f64()
        );
    }
}

fn show_run_result(report: RunReport) -> Result<()> {
    if let Some(compilation) = report.compilation {
        eprint!("{}", compilation.output.stderr);
        eprint!("{}", compilation.output.stdout);
        if compilation.output.timed_out {
            anyhow::bail!(
                "Compilation timed out after {:.3} seconds.",
                compilation.timeout.as_secs_f64()
            );
        }
        if !compilation.output.success {
            anyhow::bail!(
                "Compilation failed (exit code: {}).",
                compilation
                    .output
                    .exit_code
                    .map_or_else(|| "unavailable".into(), |code| code.to_string())
            );
        }
    }
    let execution = report
        .execution
        .expect("successful compilation must execute program");
    if !execution.success {
        match execution.exit_code {
            Some(code) => anyhow::bail!("Program exited with code {code}."),
            None => anyhow::bail!("Program terminated (exit code unavailable)."),
        }
    }
    info!("Program exited with code 0.");
    Ok(())
}

pub(crate) async fn dispatch(cli: Cli, application: &Application) -> Result<()> {
    match cli.command {
        Commands::Login { overwrite } => match application.login(overwrite, show_event).await? {
            LoginOutcome::LoggedIn { username } => info!("Logged in as {username}"),
            LoginOutcome::AlreadyLoggedIn { username } => {
                warn!("Existing REVEL_SESSION found. Use --overwrite to replace it.");
                info!("You are already logged in as {username}");
            }
            LoginOutcome::ExistingSessionInvalid => {
                warn!("Existing REVEL_SESSION found. Use --overwrite to replace it.");
                warn!("Existing REVEL_SESSION is invalid. Please logout then login or overwrite.");
            }
        },
        Commands::Logout => {
            application.logout(show_event)?;
            info!("Logged out successfully.");
        }
        Commands::Whoami => match application.whoami().await? {
            SessionStatus::LoggedIn { username } => info!("You are logged in as {username}"),
            SessionStatus::Invalid => {
                warn!("Existing REVEL_SESSION is invalid. Please logout then login or overwrite.")
            }
            SessionStatus::LoggedOut => warn!("You are not logged in."),
        },
        Commands::Download {
            contest_id,
            template_name,
            no_template,
        } => {
            let outcome = application
                .download(
                    &contest_id,
                    template_name.as_deref(),
                    no_template,
                    show_event,
                )
                .await?;
            let _ = outcome.path;
        }
        Commands::Test { case, metrics } => {
            show_test_results(&application.test(case).await?, metrics)
        }
        Commands::Run { input } => show_run_result(application.run(input).await?)?,
        Commands::Submit { no_test } => {
            let outcome = application.submit(no_test, show_event).await?;
            info!("Submit URL: {}", outcome.submission_url);
        }
        Commands::Template { action } => match action {
            TemplateCommand::List => show_templates(&application.list_templates()?),
            TemplateCommand::Show { name } => show_template(&application.show_template(&name)?),
            TemplateCommand::SetDefault { name } => {
                application.set_default_template(&name)?;
                info!("Template '{name}' is now the default.");
            }
            TemplateCommand::New {
                name,
                submit_file,
                exec_command,
                compile_command,
                pre_submit,
                default,
            } => {
                info!("Creating new template '{name}'...");
                let outcome = application.create_template(NewTemplate {
                    name: &name,
                    submit_file: &submit_file,
                    exec_command: &exec_command,
                    compile_command: compile_command.as_deref(),
                    pre_submit: pre_submit.as_deref(),
                    default,
                })?;
                info!("Template '{name}' created.");
                info!("Template directory: {}", outcome.path.display());
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Commands {
        Cli::try_parse_from(args).unwrap().command
    }

    #[test]
    fn run_reports_nonzero_exit() {
        let output = CommandOutput {
            success: false,
            timed_out: false,
            exit_code: Some(7),
            stdout: String::new(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
            real_time: Duration::ZERO,
            cpu_user_time: None,
            cpu_system_time: None,
            peak_memory_bytes: None,
        };
        let error = show_run_result(RunReport {
            compilation: None,
            execution: Some(output.clone()),
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "Program exited with code 7.");
        let error = show_run_result(RunReport {
            compilation: None,
            execution: Some(CommandOutput {
                exit_code: None,
                ..output
            }),
        })
        .unwrap_err();
        assert_eq!(error.to_string(), "Program terminated (exit code unavailable).");
    }

    #[test]
    fn parses_version() {
        let err = Cli::try_parse_from(["ackit", "-V"]).err().unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);

        let err = Cli::try_parse_from(["ackit", "--version"]).err().unwrap();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn adds_timeout_diagnostic_without_changing_raw_stderr() {
        let output = CommandOutput {
            success: false,
            timed_out: true,
            exit_code: None,
            real_time: Duration::ZERO,
            cpu_user_time: None,
            cpu_system_time: None,
            peak_memory_bytes: None,
            stdout: String::new(),
            stderr: "partial error".into(),
            stdout_truncated: false,
            stderr_truncated: false,
        };
        assert_eq!(
            stderr_with_timeout(&output, Duration::from_secs(1)),
            "Command timed out after 1.000 seconds.\npartial error"
        );
        assert_eq!(output.stderr, "partial error");
        assert_eq!(output.exit_code, None);

        let output = CommandOutput {
            stderr: String::new(),
            ..output
        };
        assert_eq!(
            stderr_with_timeout(&output, Duration::from_millis(2500)),
            "Command timed out after 2.500 seconds."
        );
    }

    #[test]
    fn parses_download_aliases_and_options() {
        assert_eq!(
            command(&["ackit", "d", "abc999", "-t", "cpp"]),
            Commands::Download {
                contest_id: "abc999".into(),
                template_name: Some("cpp".into()),
                no_template: false,
            }
        );
        assert_eq!(
            Cli::try_parse_from(["ackit", "d", "abc999", "-t", "cpp", "--no-template"])
                .err()
                .unwrap()
                .kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
        assert!(matches!(
            command(&["ackit", "n", "abc999"]),
            Commands::Download { .. }
        ));
    }

    #[test]
    fn parses_test_submit_and_template_commands() {
        assert_eq!(command(&["ackit", "run"]), Commands::Run { input: None });
        assert_eq!(
            command(&["ackit", "run", "--input", "sample.txt"]),
            Commands::Run {
                input: Some(PathBuf::from("sample.txt"))
            }
        );
        for subcommand in ["test", "t"] {
            assert_eq!(
                command(&["ackit", subcommand, "--case", "2", "--metrics"]),
                Commands::Test {
                    case: NonZeroUsize::new(2),
                    metrics: true
                }
            );
            assert!(Cli::try_parse_from(["ackit", subcommand, "--case", "0"]).is_err());
            assert!(Cli::try_parse_from(["ackit", subcommand, "--case", "abc"]).is_err());
        }
        assert_eq!(
            command(&["ackit", "t"]),
            Commands::Test {
                case: None,
                metrics: false
            }
        );
        assert_eq!(
            command(&["ackit", "s", "-n"]),
            Commands::Submit { no_test: true }
        );
        assert!(matches!(
            command(&[
                "ackit",
                "template",
                "new",
                "rust",
                "src/main.rs",
                "cargo run",
                "-c",
                "cargo build",
                "-p",
                "cargo fmt",
                "-d",
            ]),
            Commands::Template { .. }
        ));
        assert_eq!(
            command(&["ackit", "template", "list"]),
            Commands::Template {
                action: TemplateCommand::List,
            }
        );
        assert_eq!(
            command(&["ackit", "template", "show", "rust"]),
            Commands::Template {
                action: TemplateCommand::Show {
                    name: "rust".into(),
                },
            }
        );
        assert_eq!(
            command(&["ackit", "template", "set-default", "rust"]),
            Commands::Template {
                action: TemplateCommand::SetDefault {
                    name: "rust".into(),
                },
            }
        );
    }

    #[test]
    fn rejects_missing_required_arguments() {
        assert!(Cli::try_parse_from(["ackit", "download"]).is_err());
        assert!(Cli::try_parse_from(["ackit", "template", "new"]).is_err());
    }
}
