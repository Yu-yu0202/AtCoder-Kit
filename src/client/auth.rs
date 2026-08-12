use crate::client::cookie::Cookie;
use crate::client::parser::parse_username;
use crate::client::{AtCoderClient, endpoints};
use anyhow::{Context, Result, bail};
use log::{info, warn};
use reqwest::StatusCode;
use rpassword::prompt_password;

pub(crate) fn prompt_revel_session() -> Result<String> {
    warn!("No REVEL_SESSION found. Please paste your REVEL_SESSION cookie value from browser.");
    info!(
        "How to: https://github.com/Yu-yu0202/AtCoder-Kit/blob/main/docs/ja/Ex01-REVEL_SESSIONの取得.md"
    );
    let token =
        prompt_password("Paste your REVEL_SESSION: ").context("Failed to read REVEL_SESSION.")?;
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!("REVEL_SESSION cannot be empty. Login cancelled.");
    }
    Ok(token)
}

impl AtCoderClient {
    pub(crate) async fn validate_session(&self) -> Result<String> {
        let page = self.get_page(&endpoints::settings()).await?;
        if page.status == StatusCode::FOUND {
            bail!("Invalid REVEL_SESSION value.");
        }
        if !page.status.is_success() {
            bail!(
                "Failed to validate session: status {}.",
                page.status.as_u16()
            );
        }
        parse_username(&page.body)
    }
}

pub(crate) async fn verify_current_session() -> Option<Result<String>> {
    let cookie = Cookie::load().ok()?;
    if cookie.revel_session.is_empty() {
        return None;
    }
    let client = match AtCoderClient::new(Some(&cookie.revel_session)) {
        Ok(client) => client,
        Err(error) => return Some(Err(error)),
    };
    Some(client.validate_session().await)
}
