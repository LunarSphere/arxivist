use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use chrono::Utc;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    config::IndexerSettings,
    extract::extract_document,
    ids::{index_s3_prefix, manifest_s3_key, tantivy_s3_prefix},
    language::detect_english,
    models::{
        CrawledPage, IndexBuildRecord, IndexBuildStats, IndexBuildStatus, IndexManifest,
        IndexRequest, IndexStartedResponse, IndexedPage,
    },
    pagerank::{compute_page_rank, page_rank_entries},
    storage::DynIndexStorage,
    tantivy_index::{INDEX_TOKENIZER, LEXICAL_RANKING, TANTIVY_VERSION, write_tantivy_index},
};

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("{0}")]
    InvalidRequest(String),
    #[error("{0}")]
    Busy(String),
    #[error("{0}")]
    NotFound(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct IndexExecution {
    pub index_build_id: String,
    pub crawl_id: String,
}

#[async_trait]
pub trait IndexRunner: Send + Sync + 'static {
    async fn run(&self, execution: IndexExecution) -> anyhow::Result<IndexBuildStats>;
}

pub struct IndexerService {
    store: DynIndexStorage,
    runner: Arc<dyn IndexRunner>,
    active_build: Arc<Mutex<Option<String>>>,
}

impl IndexerService {
    pub fn new(store: DynIndexStorage, runner: Arc<dyn IndexRunner>) -> Self {
        Self {
            store,
            runner,
            active_build: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_index(
        &self,
        request: IndexRequest,
    ) -> Result<IndexStartedResponse, ServiceError> {
        let crawl_id = validate_request(request)?;
        let index_build_id = Uuid::new_v4().to_string();

        {
            let mut active = self.active_build.lock().await;
            if let Some(active_id) = active.as_ref() {
                return Err(ServiceError::Busy(format!(
                    "index build {active_id} is already running"
                )));
            }
            *active = Some(index_build_id.clone());
        }

        let record = IndexBuildRecord {
            index_build_id: index_build_id.clone(),
            crawl_id: crawl_id.clone(),
            status: IndexBuildStatus::Queued,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            index_s3_prefix: None,
            manifest_s3_key: None,
            pages_seen: 0,
            pages_indexed: 0,
            pages_skipped_non_english: 0,
            pages_skipped_short: 0,
            pages_failed: 0,
            error: None,
        };

        if let Err(err) = self.store.put_build(record.clone()).await {
            self.clear_active(&index_build_id).await;
            return Err(ServiceError::Internal(err));
        }

        self.spawn_build(record, crawl_id);

        Ok(IndexStartedResponse {
            index_build_id,
            status: IndexBuildStatus::Queued,
        })
    }

    pub async fn get_index(&self, index_build_id: &str) -> Result<IndexBuildRecord, ServiceError> {
        self.store.get_build(index_build_id).await?.ok_or_else(|| {
            ServiceError::NotFound(format!("index build {index_build_id} not found"))
        })
    }

    fn spawn_build(&self, mut record: IndexBuildRecord, crawl_id: String) {
        let store = self.store.clone();
        let runner = self.runner.clone();
        let active = self.active_build.clone();
        let index_build_id = record.index_build_id.clone();

        tokio::spawn(async move {
            record.status = IndexBuildStatus::Running;
            record.started_at = Some(Utc::now());
            record.error = None;
            if let Err(err) = store.put_build(record.clone()).await {
                tracing::error!(index_build_id = %index_build_id, error = %err, "failed to mark index build running");
                clear_active(active, &index_build_id).await;
                return;
            }

            let execution = IndexExecution {
                index_build_id: index_build_id.clone(),
                crawl_id,
            };

            match runner.run(execution).await {
                Ok(stats) => {
                    record.status = IndexBuildStatus::Completed;
                    record.finished_at = Some(Utc::now());
                    record.index_s3_prefix = stats.index_s3_prefix;
                    record.manifest_s3_key = stats.manifest_s3_key;
                    record.pages_seen = stats.pages_seen;
                    record.pages_indexed = stats.pages_indexed;
                    record.pages_skipped_non_english = stats.pages_skipped_non_english;
                    record.pages_skipped_short = stats.pages_skipped_short;
                    record.pages_failed = stats.pages_failed;
                    record.error = None;
                }
                Err(err) => {
                    record.status = IndexBuildStatus::Failed;
                    record.finished_at = Some(Utc::now());
                    record.error = Some(err.to_string());
                }
            }

            if let Err(err) = store.put_build(record).await {
                tracing::error!(index_build_id = %index_build_id, error = %err, "failed to persist index build result");
            }
            clear_active(active, &index_build_id).await;
        });
    }

    async fn clear_active(&self, index_build_id: &str) {
        clear_active(self.active_build.clone(), index_build_id).await;
    }
}

pub struct IndexBuildRunner {
    store: DynIndexStorage,
    settings: IndexerSettings,
}

impl IndexBuildRunner {
    pub fn new(store: DynIndexStorage, settings: IndexerSettings) -> Self {
        Self { store, settings }
    }
}

#[async_trait]
impl IndexRunner for IndexBuildRunner {
    async fn run(&self, execution: IndexExecution) -> anyhow::Result<IndexBuildStats> {
        let created_at = Utc::now();
        let pages = self.store.list_crawl_pages(&execution.crawl_id).await?;
        let mut stats = IndexBuildStats {
            pages_seen: pages.len() as u64,
            ..IndexBuildStats::default()
        };

        let mut indexed_pages = Vec::new();
        for page in pages {
            match load_indexed_page(self.store.clone(), &self.settings, page).await {
                PageLoadResult::Indexed(page) => indexed_pages.push(*page),
                PageLoadResult::SkippedShort => stats.pages_skipped_short += 1,
                PageLoadResult::SkippedNonEnglish => stats.pages_skipped_non_english += 1,
                PageLoadResult::Failed(error) => {
                    stats.pages_failed += 1;
                    tracing::warn!(error = %error, "failed to load page for indexing");
                }
            }
        }

        let ranks = compute_page_rank(
            &indexed_pages,
            self.settings.pagerank_damping,
            self.settings.pagerank_iterations,
        );
        for page in &mut indexed_pages {
            page.page_rank = ranks.get(&page.url_hash).copied().unwrap_or(1.0);
        }

        let artifact_prefix = index_s3_prefix(&execution.crawl_id, &execution.index_build_id);
        let tantivy_prefix = tantivy_s3_prefix(&execution.crawl_id, &execution.index_build_id);
        let manifest_key = manifest_s3_key(&execution.crawl_id, &execution.index_build_id);
        let work_dir = build_work_dir(
            &self.settings.work_dir,
            &execution.crawl_id,
            &execution.index_build_id,
        );

        let indexed_at = Utc::now().to_rfc3339();
        let index_pages = indexed_pages.clone();
        let index_dir = work_dir.clone();
        tokio::task::spawn_blocking(move || {
            if index_dir.exists() {
                std::fs::remove_dir_all(&index_dir)?;
            }
            write_tantivy_index(&index_dir, &index_pages, &indexed_at)
        })
        .await??;

        self.store
            .upload_index_dir(&tantivy_prefix, &work_dir)
            .await?;

        let finished_at = Utc::now();
        let manifest = IndexManifest {
            index_build_id: execution.index_build_id,
            crawl_id: execution.crawl_id,
            index_s3_prefix: artifact_prefix.clone(),
            created_at,
            finished_at,
            pages_seen: stats.pages_seen,
            pages_indexed: indexed_pages.len() as u64,
            pages_skipped_non_english: stats.pages_skipped_non_english,
            pages_skipped_short: stats.pages_skipped_short,
            pages_failed: stats.pages_failed,
            tokenizer: INDEX_TOKENIZER.to_string(),
            lexical_ranking: LEXICAL_RANKING.to_string(),
            language_confidence: self.settings.language_confidence,
            min_text_chars: self.settings.min_text_chars,
            pagerank_damping: self.settings.pagerank_damping,
            pagerank_iterations: self.settings.pagerank_iterations,
            tantivy_version: TANTIVY_VERSION.to_string(),
            page_ranks: page_rank_entries(&indexed_pages),
        };

        self.store.put_manifest(&manifest_key, &manifest).await?;

        stats.pages_indexed = indexed_pages.len() as u64;
        stats.index_s3_prefix = Some(artifact_prefix);
        stats.manifest_s3_key = Some(manifest_key);

        Ok(stats)
    }
}

enum PageLoadResult {
    Indexed(Box<IndexedPage>),
    SkippedShort,
    SkippedNonEnglish,
    Failed(anyhow::Error),
}

async fn load_indexed_page(
    store: DynIndexStorage,
    settings: &IndexerSettings,
    page: CrawledPage,
) -> PageLoadResult {
    if page.status != "fetched" {
        return PageLoadResult::Failed(anyhow::anyhow!("page {} is not fetched", page.url));
    }

    let Some(s3_key) = page.s3_key.clone() else {
        return PageLoadResult::Failed(anyhow::anyhow!("page {} has no s3_key", page.url));
    };

    let html = match store.get_raw_html(&s3_key).await {
        Ok(html) => html,
        Err(err) => return PageLoadResult::Failed(err),
    };
    let extracted = extract_document(&html);

    if extracted.body.chars().count() < settings.min_text_chars {
        return PageLoadResult::SkippedShort;
    }

    let language = detect_english(&extracted.body, settings.language_confidence);
    if !language.is_english {
        return PageLoadResult::SkippedNonEnglish;
    }

    PageLoadResult::Indexed(Box::new(IndexedPage {
        crawl_id: page.crawl_id,
        url_hash: page.url_hash,
        url: page.url,
        title: page.title.or(extracted.title),
        body: extracted.body,
        text_preview: extracted.text_preview,
        s3_key,
        content_hash: page.content_hash,
        links: page.links,
        word_count: extracted.word_count,
        page_rank: 0.0,
    }))
}

fn validate_request(request: IndexRequest) -> Result<String, ServiceError> {
    let crawl_id = request.crawl_id.trim();
    if crawl_id.is_empty() {
        return Err(ServiceError::InvalidRequest(
            "crawl_id must not be empty".to_string(),
        ));
    }
    Ok(crawl_id.to_string())
}

fn build_work_dir(base: &std::path::Path, crawl_id: &str, index_build_id: &str) -> PathBuf {
    base.join(crawl_id).join(index_build_id)
}

async fn clear_active(active: Arc<Mutex<Option<String>>>, index_build_id: &str) {
    let mut active = active.lock().await;
    if active.as_deref() == Some(index_build_id) {
        *active = None;
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tokio::sync::Notify;

    use super::*;
    use crate::storage::InMemoryIndexStorage;

    struct WaitingRunner {
        started: Arc<Notify>,
        finish: Arc<Notify>,
    }

    #[async_trait]
    impl IndexRunner for WaitingRunner {
        async fn run(&self, _execution: IndexExecution) -> anyhow::Result<IndexBuildStats> {
            self.started.notify_waiters();
            self.finish.notified().await;
            Ok(IndexBuildStats {
                pages_seen: 1,
                pages_indexed: 1,
                ..IndexBuildStats::default()
            })
        }
    }

    #[tokio::test]
    async fn rejects_empty_crawl_id() {
        let store = Arc::new(InMemoryIndexStorage::new());
        let runner = Arc::new(WaitingRunner {
            started: Arc::new(Notify::new()),
            finish: Arc::new(Notify::new()),
        });
        let service = IndexerService::new(store, runner);

        let err = service
            .start_index(IndexRequest {
                crawl_id: " ".to_string(),
            })
            .await
            .unwrap_err();

        assert!(matches!(err, ServiceError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn allows_only_one_active_index_build() {
        let store = Arc::new(InMemoryIndexStorage::new());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let runner = Arc::new(WaitingRunner {
            started: started.clone(),
            finish: finish.clone(),
        });
        let service = IndexerService::new(store, runner);

        service
            .start_index(IndexRequest {
                crawl_id: "crawl-1".to_string(),
            })
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();

        let err = service
            .start_index(IndexRequest {
                crawl_id: "crawl-2".to_string(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ServiceError::Busy(_)));

        finish.notify_waiters();
    }

    #[tokio::test]
    async fn runner_builds_index_and_manifest_from_crawler_outputs() {
        let store = Arc::new(InMemoryIndexStorage::new());
        let tempdir = tempfile::tempdir().unwrap();
        let url = "https://example.com/a";
        let raw_key = "crawl/raw/a.html";
        let body = "This article is written in plain English for readers who want to learn about search engines. The page explains how a crawler visits websites, saves documents, follows links, and prepares useful text for indexing. It uses common English words, complete sentences, and enough surrounding context for a language detector to recognize the language with confidence.";

        store
            .insert_page(CrawledPage {
                crawl_id: "crawl-1".to_string(),
                url_hash: crate::ids::url_hash(url),
                url: url.to_string(),
                status: "fetched".to_string(),
                http_status: Some(200),
                content_type: Some("text/html".to_string()),
                title: Some("Example".to_string()),
                s3_key: Some(raw_key.to_string()),
                content_hash: Some("content-hash".to_string()),
                links: Vec::new(),
                word_count: None,
                error: None,
            })
            .await;
        store
            .insert_raw_html(
                raw_key,
                format!("<html><head><title>Example</title></head><body>{body}</body></html>"),
            )
            .await;

        let runner = IndexBuildRunner::new(
            store.clone(),
            IndexerSettings {
                work_dir: tempdir.path().to_path_buf(),
                min_text_chars: 100,
                language_confidence: 0.0,
                pagerank_damping: 0.85,
                pagerank_iterations: 20,
            },
        );

        let stats = runner
            .run(IndexExecution {
                index_build_id: "build-1".to_string(),
                crawl_id: "crawl-1".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(stats.pages_seen, 1);
        assert_eq!(stats.pages_indexed, 1);
        assert_eq!(stats.pages_failed, 0);

        let manifest_key = stats.manifest_s3_key.unwrap();
        let manifest = store.manifest(&manifest_key).await.unwrap();
        assert_eq!(manifest.pages_indexed, 1);
        assert_eq!(manifest.page_ranks.len(), 1);
        assert_eq!(manifest.page_ranks[0].page_rank, 1.0);
    }
}
