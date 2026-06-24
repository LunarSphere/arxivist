// define arguments for running in cli
use clap::Parser;
use std::path::PathBuf;
use url::Url;

#[derive(Debug, Parser)]
pub struct Args {
    #[arg(long, value_enum, default_value_t = StorageMode::Local, env = "ARXIVIST_STORAGE_MODE")]
    pub storage: StorageMode,
    #[arg(long = "seed")]
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
    #[arg(long, env = "ARXIVIST_DATA_BUCKET")]
    pub data_bucket: Option<String>,
    #[arg(long, env = "ARXIVIST_PAGES_TABLE")]
    pub pages_table: Option<String>,
    #[arg(long, env = "ARXIVIST_CRAWL_URLS_TABLE")]
    pub crawl_urls_table: Option<String>,
    #[arg(long, env = "ARXIVIST_CRAWL_QUEUE_URL")]
    pub crawl_queue_url: Option<String>,
    #[arg(long, default_value_t = 10, env = "ARXIVIST_EMPTY_RECEIVE_LIMIT")]
    pub empty_receive_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum StorageMode {
    Local,
    Aws,
}
