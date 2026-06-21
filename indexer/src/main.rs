use std::sync::Arc;

use anyhow::Context;
use arxivist_indexer::{
    AwsSettings, IndexerService, IndexerSettings, ServerSettings, api::router, aws::AwsIndexStore,
    service::IndexBuildRunner,
};
use tokio::net::TcpListener;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let server = ServerSettings::from_env()?;
    let aws = AwsSettings::from_env()?;
    let indexer = IndexerSettings::from_env()?;

    let store = Arc::new(AwsIndexStore::from_settings(aws).await?);
    let runner = Arc::new(IndexBuildRunner::new(store.clone(), indexer));
    let service = Arc::new(IndexerService::new(store, runner));

    let app = router(service).layer(TraceLayer::new_for_http());
    let listener = TcpListener::bind(server.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", server.bind_addr))?;

    tracing::info!(addr = %server.bind_addr, "indexer service listening");
    axum::serve(listener, app).await?;

    Ok(())
}
