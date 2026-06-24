mod aws_indexer;
mod index;
mod pagerank;
mod records;

use anyhow::{Result, anyhow};
use clap::Parser;
use std::{fs, path::PathBuf};
use tracing::info;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, value_enum, default_value_t = StorageMode::Local, env = "ARXIVIST_STORAGE_MODE")]
    storage: StorageMode,
    #[arg(long, default_value = "data/dev/crawl/pages.jsonl")]
    crawl_records: PathBuf,
    #[arg(long, default_value = "data/dev/index/index.json")]
    output: PathBuf,
    #[arg(long, env = "ARXIVIST_DATA_BUCKET")]
    data_bucket: Option<String>,
    #[arg(long, env = "ARXIVIST_PAGES_TABLE")]
    pages_table: Option<String>,
    #[arg(
        long,
        default_value = "indexes/active/index.json",
        env = "ARXIVIST_ACTIVE_INDEX_KEY"
    )]
    active_index_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum StorageMode {
    Local,
    Aws,
}

#[tokio::main]
async fn main() -> Result<()> {
    // logs in terminal
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = Args::parse();
    if args.storage == StorageMode::Aws {
        return aws_indexer::run(&args).await;
    }

    let records = records::read_records(&args.crawl_records)?;
    let page_ranks = pagerank::compute_page_rank(&records, 0.85, 20);
    let index = index::build_index(records, page_ranks);

    if let Some(parent) = args.output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&args.output, serde_json::to_vec_pretty(&index)?)?;
    info!(path = %args.output.display(), docs = index.documents.len(), "wrote index");
    Ok(())
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{name} is required when --storage aws is used"))
}
