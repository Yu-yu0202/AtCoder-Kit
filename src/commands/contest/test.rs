use crate::core::test::{TestStatus, test as run_test};
use anyhow::*;
use colored::Colorize;
use log::*;

pub async fn test() -> Result<()> {
    let results = run_test().await?;

    for result in &results {
        match result.status {
            TestStatus::AC => {
                info!("{}", "AC".green().bold());
            }
            TestStatus::WA => {
                warn!("{}", "Wrong Answer".red().bold());
                info!("expected:\n{}", result.expected);
                info!("got:\n{}", result.stdout);
            }
            TestStatus::RE => {
                warn!("{}", "Runtime Error".red().bold());
                info!("exit code: {}", result.exit_code);
                info!("stderr:\n{}", result.stderr);
            }
            TestStatus::CE => {
                warn!("{}", "Compile Error".red().bold());
                info!("compiler exit code: {}", result.exit_code);
                info!("stderr:\n{}", result.stderr);
                info!("stdout:\n{}", result.stdout);
            }
        }
    }

    Ok(())
}
