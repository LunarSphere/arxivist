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
    #[arg(long, default_value = "data/dev/index")]
    output: PathBuf,
}

#[tokio::main]
async fn main() -> Result<()> {
    // logs in terminal
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = Args::parse();
    let records = records::read_records(&args.crawl_records)?;
    let page_ranks = pagerank::compute_page_rank(&records, 0.85, 20);
    let index = index::build_index(records, page_ranks);

    if args
        .output
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        if let Some(parent) = args.output.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&args.output, serde_json::to_vec_pretty(&index)?)?;
        info!(path = %args.output.display(), docs = index.documents.len(), "wrote legacy index");
    } else {
        fs::create_dir_all(&args.output)?;
        let version = index::version_from_time();
        index::write_sharded_index(&index, &args.output, &version)?;
        info!(
            path = %args.output.display(),
            version,
            docs = index.documents.len(),
            terms = index.terms.len(),
            "wrote sharded index"
        );
    }
    Ok(())
}
