mod args;
mod extract;
mod filters;
mod record;
mod runner;
mod spider_client;
mod types;

use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = args::Args::parse();
    runner::run(args).await
}
