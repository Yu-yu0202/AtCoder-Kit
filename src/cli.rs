use crate::application::sample::{TestResult, TestStatus};
use crate::application::{AppEvent, Application, LoginOutcome, SessionStatus};
use crate::workspace::template::NewTemplate;
use anyhow::Result;
use clap::{Parser, Subcommand};
use colored::Colorize;
use log::{info, warn};

#[derive(Parser)]
#[command(name = "ackit", about = format!("{}", "AtCoder-Kit".green().bold()))]
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
        template_name: Option<String>,
        /// Skip clone default template
        #[arg(short, long)]
        no_template: bool,
    },
    /// Test the program with sample cases
    #[command(visible_alias = "t")]
    Test,
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

fn show_test_results(results: &[TestResult]) {
    for result in results {
        match result.status {
            TestStatus::Ac => info!("{}", "AC".green().bold()),
            TestStatus::Wa => {
                warn!("{}", "Wrong Answer".red().bold());
                info!("expected:\n{}", result.expected);
                info!("got:\n{}", result.stdout);
            }
            TestStatus::Re => {
                warn!("{}", "Runtime Error".red().bold());
                info!("exit code: {}", result.exit_code);
                info!("stderr:\n{}", result.stderr);
            }
            TestStatus::Tle => {
                warn!("{}", "Time Limit Exceeded".red().bold());
                info!("stderr:\n{}", result.stderr);
            }
            TestStatus::Ole => {
                warn!("{}", "Output Limit Exceeded".red().bold());
                info!("stdout:\n{}", result.stdout);
                info!("stderr:\n{}", result.stderr);
            }
            TestStatus::Ce => {
                warn!("{}", "Compile Error".red().bold());
                info!("compiler exit code: {}", result.exit_code);
                info!("stderr:\n{}", result.stderr);
                info!("stdout:\n{}", result.stdout);
            }
        }
    }
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
        Commands::Test => show_test_results(&application.test().await?),
        Commands::Submit { no_test } => {
            let outcome = application.submit(no_test, show_event).await?;
            info!("Submit URL: {}", outcome.submission_url);
        }
        Commands::Template {
            action:
                TemplateCommand::New {
                    name,
                    submit_file,
                    exec_command,
                    compile_command,
                    pre_submit,
                    default,
                },
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
    fn parses_download_aliases_and_options() {
        assert_eq!(
            command(&["ackit", "d", "abc999", "cpp", "--no-template"]),
            Commands::Download {
                contest_id: "abc999".into(),
                template_name: Some("cpp".into()),
                no_template: true,
            }
        );
        assert!(matches!(
            command(&["ackit", "n", "abc999"]),
            Commands::Download { .. }
        ));
    }

    #[test]
    fn parses_test_submit_and_template_commands() {
        assert_eq!(command(&["ackit", "t"]), Commands::Test);
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
    }

    #[test]
    fn rejects_missing_required_arguments() {
        assert!(Cli::try_parse_from(["ackit", "download"]).is_err());
        assert!(Cli::try_parse_from(["ackit", "template", "new"]).is_err());
    }
}
