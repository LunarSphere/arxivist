use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::models::{CrawlRecord, PageRecord};

pub type DynStorage = Arc<dyn CrawlStorage>;

#[async_trait]
pub trait CrawlStorage: Send + Sync + 'static {
    async fn put_crawl(&self, record: CrawlRecord) -> Result<()>;
    async fn get_crawl(&self, crawl_id: &str) -> Result<Option<CrawlRecord>>;
    async fn put_page(&self, record: PageRecord) -> Result<()>;
    async fn put_raw_html(&self, crawl_id: &str, url_hash: &str, html: String) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct InMemoryStorage {
    crawls: RwLock<HashMap<String, CrawlRecord>>,
    pages: RwLock<HashMap<(String, String), PageRecord>>,
    raw_html: RwLock<HashMap<String, String>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn pages(&self) -> Vec<PageRecord> {
        self.pages.read().await.values().cloned().collect()
    }
}

#[async_trait]
impl CrawlStorage for InMemoryStorage {
    async fn put_crawl(&self, record: CrawlRecord) -> Result<()> {
        self.crawls
            .write()
            .await
            .insert(record.crawl_id.clone(), record);
        Ok(())
    }

    async fn get_crawl(&self, crawl_id: &str) -> Result<Option<CrawlRecord>> {
        Ok(self.crawls.read().await.get(crawl_id).cloned())
    }

    async fn put_page(&self, record: PageRecord) -> Result<()> {
        self.pages
            .write()
            .await
            .insert((record.crawl_id.clone(), record.url_hash.clone()), record);
        Ok(())
    }

    async fn put_raw_html(&self, crawl_id: &str, url_hash: &str, html: String) -> Result<String> {
        let key = crate::ids::raw_html_s3_key(crawl_id, url_hash);
        self.raw_html.write().await.insert(key.clone(), html);
        Ok(key)
    }
}
