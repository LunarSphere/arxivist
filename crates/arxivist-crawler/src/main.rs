mod args;
mod aws_runner;
mod extract;
mod filters;
mod record;
mod runner;
mod spider_client;
mod types;

use anyhow::Result;
use clap::Parser;

#[tokio::main]
// runs our webcrawler with ability to toggle Local and AWS storage
async fn main() -> Result<()> {
    // log ouputs for debugging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = args::Args::parse();
    match args.storage {
        args::StorageMode::Local => runner::run(args).await, // this is a bassically a fancy "if this then that" statement
        args::StorageMode::Aws => aws_runner::run(args).await,
    }
}
