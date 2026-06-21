use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IndexRequest {
    pub crawl_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexStartedResponse {
    pub index_build_id: String,
    pub status: IndexBuildStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexBuildRecord {
    pub index_build_id: String,
    pub crawl_id: String,
    pub status: IndexBuildStatus,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub index_s3_prefix: Option<String>,
    pub manifest_s3_key: Option<String>,
    pub pages_seen: u64,
    pub pages_indexed: u64,
    pub pages_skipped_non_english: u64,
    pub pages_skipped_short: u64,
    pub pages_failed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IndexBuildStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexBuildStats {
    pub pages_seen: u64,
    pub pages_indexed: u64,
    pub pages_skipped_non_english: u64,
    pub pages_skipped_short: u64,
    pub pages_failed: u64,
    pub index_s3_prefix: Option<String>,
    pub manifest_s3_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrawledPage {
    pub crawl_id: String,
    pub url_hash: String,
    pub url: String,
    pub status: String,
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub title: Option<String>,
    pub s3_key: Option<String>,
    pub content_hash: Option<String>,
    pub links: Vec<String>,
    pub word_count: Option<usize>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexedPage {
    pub crawl_id: String,
    pub url_hash: String,
    pub url: String,
    pub title: Option<String>,
    pub body: String,
    pub text_preview: String,
    pub s3_key: String,
    pub content_hash: Option<String>,
    pub links: Vec<String>,
    pub word_count: usize,
    pub page_rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PageRankEntry {
    pub url_hash: String,
    pub url: String,
    pub page_rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IndexManifest {
    pub index_build_id: String,
    pub crawl_id: String,
    pub index_s3_prefix: String,
    pub created_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub pages_seen: u64,
    pub pages_indexed: u64,
    pub pages_skipped_non_english: u64,
    pub pages_skipped_short: u64,
    pub pages_failed: u64,
    pub tokenizer: String,
    pub lexical_ranking: String,
    pub language_confidence: f64,
    pub min_text_chars: usize,
    pub pagerank_damping: f64,
    pub pagerank_iterations: usize,
    pub tantivy_version: String,
    pub page_ranks: Vec<PageRankEntry>,
}
