use clap::Parser;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(long = "seed", required = true)]
    pub seeds: Vec<Url>,
    #[arg(long, default_value_t = 100)]
    pub max_pages: usize,
    #[arg(long, default_value_t = 2)]
    pub max_depth: usize,
    #[arg(long, default_value = "data/dev/crawl")]
    pub output_dir: PathBuf,
    #[arg(long, default_value_t = 700)]
    pub delay_ms: u64,
    #[arg(long, default_value_t = 4)]
    pub concurrency: usize,
    #[arg(long, default_value_t = 3)]
    pub bad_host_threshold: usize,
}
