use reqwest::{Client, Request, Response, redirect::Policy};
use reqwest_middleware::{ClientWithMiddleware, Middleware, Next, Result};
use std::sync::LazyLock;
use std::time::Duration;
use tokio::time::sleep;

struct SleepMiddleware(Duration);

#[async_trait::async_trait]
impl Middleware for SleepMiddleware {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut http::Extensions,
        next: Next<'_>,
    ) -> Result<Response> {
        sleep(self.0).await;
        next.run(req, extensions).await
    }
}

pub(crate) static CLIENT: LazyLock<ClientWithMiddleware> = LazyLock::new(|| {
    let client = Client::builder()
        .cookie_store(true)
        .redirect(Policy::none())
        .user_agent(format!("AtCoder-Kit/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .expect("Failed to build HTTP client.");

    reqwest_middleware::ClientBuilder::new(client)
        .with(SleepMiddleware(Duration::from_millis(250)))
        .build()
});
