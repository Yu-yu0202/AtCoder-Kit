mod client;
mod commands;
mod core;

use anyhow::*;
use clap::{Parser, Subcommand};
use colored::*;
use log::*;
use std::result::Result::{Err, Ok};

use crate::commands::contest::submit::submit;
use commands::{
    contest::download::download,
    contest::test::test,
    session::{login::login, logout::logout, whoami::whoami},
    template::new::new,
};
use core::logger::init_logger;

macro_rules! command {
    ($cmd:expr, $(
        $variant:pat => $handler:expr
    ),* $(,)?) => {
        match $cmd {
            $(
                $variant => $handler,
            )*
        }
    };
}

static APP_NAME: &'static str = "atcoder-kit";

#[derive(Parser)]
#[command(name = "ackit",
	about = format!("{}", "AtCoder-Kit".green().bold()))]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Login to atcoder.jp
    Login {
        /// Overwrite existing REVEL_SESSION cookie value
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
        /// Contest ID
        /// (ex. abc001, ahc001, awc0001)
        contest_id: String,
        /// Template name
        template_name: Option<String>,
        /// Skip clone template
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
        action: Template,
    },
}

#[derive(Subcommand)]
enum Template {
    /// Create new template files
    New {
        /// Template name
        name: String,

        /// The source code file
        /// (ex. main.py, main.cpp, src/main.rs)
        submit_file: String,
        /// The executable file or command to run the program
        /// (ex. "python3 main.py", a.out, target/debug/a)
        exec_command: String,
        /// Compiler Command (for compile language)
        /// (ex. "g++ main.cpp -O3", "cargo build")
        #[arg(short, long)]
        compile_command: Option<String>,
        /// Pre-submit Command
        /// (ex. execute bundler)
        #[arg(short, long)]
        pre_submit: Option<String>,
        /// Set as default template
        #[arg(short, long)]
        is_default: bool,
    },
}

fn init() -> Cli {
    init_logger();
    Cli::parse()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if let Err(e) = run(init()).await {
        error!("{e}");
        std::process::exit(1);
    }
}

async fn run(args: Cli) -> Result<()> {
    command!(args.command,
        Commands::Login { overwrite } => login(overwrite).await,
        Commands::Logout => logout().await,
        Commands::Whoami => whoami().await,
        Commands::Download { contest_id, template_name, no_template } => download(&*contest_id, template_name.as_deref(), no_template).await,
        Commands::Test => test().await,
        Commands::Submit { no_test } => submit(no_test).await,
        Commands::Template { action } => {
            command!(action,
                Template::New { name, submit_file, exec_command, compile_command, pre_submit, is_default } => new(&*name, &*submit_file, &*exec_command, compile_command.as_deref(), pre_submit.as_deref(), is_default).await
            )
        }
    )?;

    Ok(())
}
