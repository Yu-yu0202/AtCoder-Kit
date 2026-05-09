use crate::client::auth::verify_current_session;
use anyhow::*;
use log::*;
use std::result::Result::{Err, Ok};

pub async fn whoami() -> Result<()> {
    if let Some(result) = verify_current_session().await {
        match result {
            Ok(username) => info!("You are logged in as {}", username),
            Err(_) => {
                warn!("Existing REVEL_SESSION is invalid. Please logout then login or overwrite.")
            }
        }
    } else {
        warn!("You are not logged in.");
    }
    Ok(())
}
