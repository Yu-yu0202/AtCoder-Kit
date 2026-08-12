use reqwest::{Client, Request, Response, redirect::Policy};
use reqwest_middleware::{ClientWithMiddleware, Middleware, Next, Result as MiddlewareResult};
use std::time::Duration;
use tokio::time::sleep;

struct SleepMiddleware(Duration);

#[async_trait::async_trait]
impl Middleware for SleepMiddleware {
    async fn handle(
        &self,
        request: Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> MiddlewareResult<Response> {
        sleep(self.0).await;
        next.run(request, extensions).await
    }
}

pub(crate) fn build_client() -> reqwest::Result<ClientWithMiddleware> {
    let client = Client::builder()
        .cookie_store(true)
        .redirect(Policy::none())
        .user_agent(format!("AtCoder-Kit/{}", env!("CARGO_PKG_VERSION")))
        .build()?;

    Ok(reqwest_middleware::ClientBuilder::new(client)
        .with(SleepMiddleware(Duration::from_millis(250)))
        .build())
}
