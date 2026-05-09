use crate::client::cookie::Cookie;
use crate::client::endpoints;
use crate::core::network::CLIENT;
use anyhow::*;
use log::*;
use reqwest::{
    StatusCode,
    header::{COOKIE, HeaderMap, HeaderValue},
};
use rpassword::prompt_password;
use scraper::{Html, Selector};
use std::result::Result::Ok;

pub fn get_revel_session() -> Result<String> {
    warn!("No REVEL_SESSION found. Please paste your REVEL_SESSION cookie value from browser.");
    info!("️How to: "); //TODO: Add docs for how to get REVEL_SESSION cookie value (Low)

    let token =
        prompt_password("Paste your REVEL_SESSION: ").context("Failed to read REVEL_SESSION.")?;
    let token = token.trim().to_string();

    if token.is_empty() {
        bail!("REVEL_SESSION cannot be empty. Login cancelled.");
    }

    Ok(token)
}

pub async fn validate_token(token: &str) -> Result<String> {
    let cookie_value = format!("REVEL_SESSION={}", token);

    let mut headers = HeaderMap::new();
    headers.insert(COOKIE, HeaderValue::from_str(&cookie_value)?);

    let res = CLIENT
        .get(endpoints::settings())
        .headers(headers)
        .send()
        .await
        .context("Failed to contact server.")?;

    if res.status() == StatusCode::FOUND {
        bail!("Invalid REVEL_SESSION value.");
    }

    let document = Html::parse_document(&res.text().await.context("Failed to parse document.")?);

    let selector = Selector::parse(r#"[id="ui.UserName"]"#)
        .map_err(|_| anyhow!("Failed to parse document."))?;

    let username = document.select(&selector)
		.next()
		.context("Failed to get username.\nMaybe AtCoder's website structure has changed?\nPlease report this issue: https://github.com/Yu-yu0202/atcoder-kit/issues")?
		.value()
		.attr("value")
		.context("Failed to get username.\nMaybe AtCoder's website structure has changed?\nPlease report this issue: https://github.com/Yu-yu0202/atcoder-kit/issues")?;

    Ok(username.to_string())
}

pub async fn verify_current_session() -> Option<Result<String>> {
    let cookie = Cookie::load().ok()?;
    if cookie.revel_session.is_empty() {
        return None;
    }
    Some(validate_token(&cookie.revel_session).await)
}

pub fn get_token() -> Result<Option<String>> {
    let cookie = Cookie::load()?;
    if cookie.revel_session.is_empty() {
        return Ok(None);
    }
    Ok(Some(cookie.revel_session))
}

pub fn get_token_header() -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();

    if let Some(token) = get_token()? {
        let cookie = format!("REVEL_SESSION={token}");
        headers.insert(COOKIE, HeaderValue::from_str(&cookie)?);
    }

    Ok(headers)
}
