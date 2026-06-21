use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CrawlRequest {
    pub seed_urls: Vec<String>,
    #[serde(default = "default_max_pages")]
    pub max_pages: u32,
    #[serde(default = "default_depth_limit")]
    pub depth_limit: usize,
}

impl CrawlRequest {
    pub fn normalized(mut self) -> Self {
        if self.max_pages == 0 {
            self.max_pages = default_max_pages();
        }
        if self.depth_limit == 0 {
            self.depth_limit = default_depth_limit();
        }
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrawlStartedResponse {
    pub crawl_id: String,
    pub status: CrawlStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrawlRecord {
    pub crawl_id: String,
    pub status: CrawlStatus,
    pub seed_urls: Vec<String>,
    pub max_pages: u32,
    pub depth_limit: usize,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub pages_fetched: u64,
    pub pages_failed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PageRecord {
    pub crawl_id: String,
    pub url_hash: String,
    pub url: String,
    pub status: PageStatus,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub title: Option<String>,
    pub s3_key: Option<String>,
    pub content_hash: Option<String>,
    pub links: Vec<String>,
    pub assets: Vec<AssetRecord>,
    pub text_preview: Option<String>,
    pub word_count: usize,
    pub error: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AssetRecord {
    pub asset_url: String,
    pub asset_type: String,
    pub alt_text: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrawlStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PageStatus {
    Fetched,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrawlStats {
    pub pages_fetched: u64,
    pub pages_failed: u64,
}

pub fn default_max_pages() -> u32 {
    5_000
}

pub fn default_depth_limit() -> usize {
    6
}
