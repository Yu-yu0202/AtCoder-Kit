mod application;
mod cli;
mod client;
mod logger;
mod network;
mod validation;
mod workspace;

use anyhow::{Context, Result};

pub(crate) const APP_NAME: &str = "atcoder-kit";

pub async fn run() -> Result<()> {
    logger::init_logger();
    let start_path = std::env::current_dir().context("Failed to get current directory.")?;
    let application = application::Application::new(start_path);
    cli::dispatch(cli::parse(), &application).await
}
