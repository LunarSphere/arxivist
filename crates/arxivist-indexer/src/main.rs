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
    let records = records::read_records(&args.crawl_records)?; // object representing a vector of crawl record structs
    let page_ranks = pagerank::compute_page_rank(&records, 0.85, 20); //compute the page rank of every page url in our crawl record 20 iterations and a damping factor of .85
    let index = index::build_index(records, page_ranks); // the indexing gets handled here. more details in index.rs

    //write legacy index
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
        // write the sharded index (smaller indexes to make indexing faster)
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
