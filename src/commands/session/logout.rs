use crate::client::cookie::Cookie;
use anyhow::*;
use log::*;

pub async fn logout() -> Result<()> {
    info!("Logging out...");

    Cookie::set_default().context("Failed to clear REVEL_SESSION.")?;

    info!("Logged out successfully.");
    Ok(())
}
