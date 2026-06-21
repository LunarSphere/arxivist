use std::sync::Arc;

use anyhow::Context;
use arxivist_crawler::{
    AwsSettings, CrawlerService, ServerSettings, SpiderSettings, api::router, aws::AwsStore,
    crawl::SpiderCrawlRunner,
};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // log to console
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let server = ServerSettings::from_env()?;
    let aws = AwsSettings::from_env()?;
    let spider = SpiderSettings::from_env()?;

    let store = Arc::new(AwsStore::from_settings(aws).await?);
    let runner = Arc::new(SpiderCrawlRunner::new(store.clone(), spider));
    let service = Arc::new(CrawlerService::new(store, runner));

    let app = router(service).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(server.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", server.bind_addr))?;

    tracing::info!(addr = %server.bind_addr, "crawler service listening");
    // wait for api request to do anything?
    axum::serve(listener, app).await?;

    Ok(())
}
