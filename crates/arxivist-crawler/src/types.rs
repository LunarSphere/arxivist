// pub structs that are specific to the crawler
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub url: Url,
    pub source_seed: Url,
    pub referrer: Option<Url>,
    pub depth: usize,
}

#[derive(Debug)]
pub struct PageSnapshot {
    pub final_url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub html: String,
}
