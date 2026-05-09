use crate::client::submit::submit as client_submit;
use crate::core::test::test as run_test;
use anyhow::*;
use log::*;

pub async fn submit(no_test: bool) -> Result<()> {
    info!("Testing...");
    let should_test = !no_test;
    if should_test {
        let result = run_test().await?;

        if result.iter().any(|r| r.is_failed()) {
            bail!("Test failed. Please fix the issues and try submitting again.");
        }
    }
    info!("Test successful.");

    info!("Submitting...");
    let result_url = client_submit().await?;
    info!("Submit successful.");

    info!("Submit URL: {}", result_url);

    Ok(())
}
