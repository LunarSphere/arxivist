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
// runs our local web crawler
async fn main() -> Result<()> {
    // log ouputs for debugging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = args::Args::parse();
    runner::run(args).await
}
