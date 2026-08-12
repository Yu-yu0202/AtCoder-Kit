pub(crate) mod auth;
pub(crate) mod contest;
pub(crate) mod cookie;
pub(crate) mod endpoints;
pub(crate) mod model;
pub(crate) mod parser;
pub(crate) mod submit;

use crate::network::build_client;
use anyhow::{Context, Result};
use reqwest::StatusCode;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use reqwest_middleware::ClientWithMiddleware;

pub(crate) struct AtCoderClient {
    http: ClientWithMiddleware,
    headers: HeaderMap,
}

pub(crate) struct HttpPage {
    pub(crate) status: StatusCode,
    pub(crate) body: String,
}

impl AtCoderClient {
    pub(crate) fn new(session: Option<&str>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        if let Some(session) = session.filter(|session| !session.is_empty()) {
            headers.insert(
                COOKIE,
                HeaderValue::from_str(&format!("REVEL_SESSION={session}"))?,
            );
        }
        let http = build_client().context("Failed to build HTTP client.")?;
        Ok(Self { http, headers })
    }

    pub(crate) fn from_stored_session() -> Result<Self> {
        let cookie = cookie::Cookie::load()?;
        Self::new((!cookie.revel_session.is_empty()).then_some(cookie.revel_session.as_str()))
    }

    pub(crate) async fn get_page(&self, url: &str) -> Result<HttpPage> {
        let response = self
            .http
            .get(url)
            .headers(self.headers.clone())
            .send()
            .await
            .context("Failed to contact server.")?;
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read server response.")?;
        Ok(HttpPage { status, body })
    }

    pub(crate) fn headers(&self) -> HeaderMap {
        self.headers.clone()
    }
}
