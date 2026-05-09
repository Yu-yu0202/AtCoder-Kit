use crate::client::contest::{Contest, save_contest};
use anyhow::*;
use log::*;

pub async fn download(
    contest_id: &str,
    template_name: Option<&str>,
    no_template: bool,
) -> Result<()> {
    info!("Fetching contest '{}'...", contest_id);
    let contest = Contest::fetch(contest_id).await?;

    info!("Saving contest to '{}'...", contest_id);

    save_contest(contest, template_name, no_template)?;

    Ok(())
}
