use crate::client::auth::{get_revel_session, validate_token, verify_current_session};
use crate::client::cookie::Cookie;
use anyhow::*;
use log::*;
use std::result::Result::{Err, Ok};

pub async fn login(overwrite: bool) -> Result<()> {
    if !overwrite {
        if let Some(result) = verify_current_session().await {
            warn!("Existing REVEL_SESSION found. Use --overwrite to replace it.");
            match result {
                Ok(username) => info!("You are already logged in as {}", username),
                Err(_) => warn!(
                    "Existing REVEL_SESSION is invalid. Please logout then login or overwrite."
                ),
            }
            return Ok(());
        }
    }

    let revel_session = get_revel_session()?;

    info!("Logging in...");

    let username = validate_token(&revel_session).await?;

    info!("Logged in as {}", username);

    Cookie::store(&Cookie { revel_session }).context("Failed to store REVEL_SESSION.")?;

    Ok(())
}
