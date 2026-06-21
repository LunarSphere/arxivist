use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::models::{CrawledPage, IndexBuildRecord, IndexManifest};

pub type DynIndexStorage = Arc<dyn IndexStorage>;

#[async_trait]
pub trait IndexStorage: Send + Sync + 'static {
    async fn put_build(&self, record: IndexBuildRecord) -> Result<()>;
    async fn get_build(&self, index_build_id: &str) -> Result<Option<IndexBuildRecord>>;
    async fn list_crawl_pages(&self, crawl_id: &str) -> Result<Vec<CrawledPage>>;
    async fn get_raw_html(&self, s3_key: &str) -> Result<String>;
    async fn upload_index_dir(&self, s3_prefix: &str, dir: &Path) -> Result<()>;
    async fn put_manifest(&self, s3_key: &str, manifest: &IndexManifest) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct InMemoryIndexStorage {
    builds: RwLock<HashMap<String, IndexBuildRecord>>,
    pages_by_crawl: RwLock<HashMap<String, Vec<CrawledPage>>>,
    raw_html: RwLock<HashMap<String, String>>,
    manifests: RwLock<HashMap<String, IndexManifest>>,
    uploaded_files: RwLock<HashMap<String, Vec<PathBuf>>>,
}

impl InMemoryIndexStorage {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn insert_page(&self, page: CrawledPage) {
        self.pages_by_crawl
            .write()
            .await
            .entry(page.crawl_id.clone())
            .or_default()
            .push(page);
    }

    pub async fn insert_raw_html(&self, s3_key: impl Into<String>, html: impl Into<String>) {
        self.raw_html
            .write()
            .await
            .insert(s3_key.into(), html.into());
    }

    pub async fn manifest(&self, s3_key: &str) -> Option<IndexManifest> {
        self.manifests.read().await.get(s3_key).cloned()
    }
}

#[async_trait]
impl IndexStorage for InMemoryIndexStorage {
    async fn put_build(&self, record: IndexBuildRecord) -> Result<()> {
        self.builds
            .write()
            .await
            .insert(record.index_build_id.clone(), record);
        Ok(())
    }

    async fn get_build(&self, index_build_id: &str) -> Result<Option<IndexBuildRecord>> {
        Ok(self.builds.read().await.get(index_build_id).cloned())
    }

    async fn list_crawl_pages(&self, crawl_id: &str) -> Result<Vec<CrawledPage>> {
        Ok(self
            .pages_by_crawl
            .read()
            .await
            .get(crawl_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_raw_html(&self, s3_key: &str) -> Result<String> {
        self.raw_html
            .read()
            .await
            .get(s3_key)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing raw HTML for key {s3_key}"))
    }

    async fn upload_index_dir(&self, s3_prefix: &str, dir: &Path) -> Result<()> {
        let files = walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.path().to_path_buf())
            .collect::<Vec<_>>();
        self.uploaded_files
            .write()
            .await
            .insert(s3_prefix.to_string(), files);
        Ok(())
    }

    async fn put_manifest(&self, s3_key: &str, manifest: &IndexManifest) -> Result<()> {
        self.manifests
            .write()
            .await
            .insert(s3_key.to_string(), manifest.clone());
        Ok(())
    }
}
