mod index;
mod pagerank;
mod records;

use anyhow::Result;
use clap::Parser;
use std::{fs, path::PathBuf};
use tracing::info;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "data/dev/crawl/pages.jsonl")]
    crawl_records: PathBuf,
    #[arg(long, default_value = "data/dev/index/index.json")]
    output: PathBuf,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = Args::parse();
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
