use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    ids::normalize_url,
    models::{CrawlRecord, CrawlRequest, CrawlStartedResponse, CrawlStats, CrawlStatus},
    storage::DynStorage,
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
pub struct CrawlExecution {
    pub crawl_id: String,
    pub request: CrawlRequest,
}

#[async_trait]
pub trait CrawlRunner: Send + Sync + 'static {
    async fn run(&self, execution: CrawlExecution) -> anyhow::Result<CrawlStats>;
}

pub struct CrawlerService {
    store: DynStorage,
    runner: Arc<dyn CrawlRunner>,
    active_crawl: Arc<Mutex<Option<String>>>,
}

impl CrawlerService {
    pub fn new(store: DynStorage, runner: Arc<dyn CrawlRunner>) -> Self {
        Self {
            store,
            runner,
            active_crawl: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_crawl(
        &self,
        request: CrawlRequest,
    ) -> Result<CrawlStartedResponse, ServiceError> {
        let request = validate_request(request)?;
        let crawl_id = Uuid::new_v4().to_string();

        {
            let mut active = self.active_crawl.lock().await;
            if let Some(active_id) = active.as_ref() {
                return Err(ServiceError::Busy(format!(
                    "crawl {active_id} is already running"
                )));
            }
            *active = Some(crawl_id.clone());
        }

        let record = CrawlRecord {
            crawl_id: crawl_id.clone(),
            status: CrawlStatus::Queued,
            seed_urls: request.seed_urls.clone(),
            max_pages: request.max_pages,
            depth_limit: request.depth_limit,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
            pages_fetched: 0,
            pages_failed: 0,
            error: None,
        };

        if let Err(err) = self.store.put_crawl(record.clone()).await {
            self.clear_active(&crawl_id).await;
            return Err(ServiceError::Internal(err));
        }

        self.spawn_crawl(record, request);

        Ok(CrawlStartedResponse {
            crawl_id,
            status: CrawlStatus::Queued,
        })
    }

    pub async fn get_crawl(&self, crawl_id: &str) -> Result<CrawlRecord, ServiceError> {
        self.store
            .get_crawl(crawl_id)
            .await?
            .ok_or_else(|| ServiceError::NotFound(format!("crawl {crawl_id} not found")))
    }

    fn spawn_crawl(&self, mut record: CrawlRecord, request: CrawlRequest) {
        let store = self.store.clone();
        let runner = self.runner.clone();
        let active = self.active_crawl.clone();
        let crawl_id = record.crawl_id.clone();

        tokio::spawn(async move {
            record.status = CrawlStatus::Running;
            record.started_at = Some(Utc::now());
            record.error = None;
            if let Err(err) = store.put_crawl(record.clone()).await {
                tracing::error!(crawl_id = %crawl_id, error = %err, "failed to mark crawl running");
                clear_active(active, &crawl_id).await;
                return;
            }

            let execution = CrawlExecution {
                crawl_id: crawl_id.clone(),
                request,
            };

            match runner.run(execution).await {
                Ok(stats) => {
                    record.status = CrawlStatus::Completed;
                    record.finished_at = Some(Utc::now());
                    record.pages_fetched = stats.pages_fetched;
                    record.pages_failed = stats.pages_failed;
                    record.error = None;
                }
                Err(err) => {
                    record.status = CrawlStatus::Failed;
                    record.finished_at = Some(Utc::now());
                    record.error = Some(err.to_string());
                }
            }

            if let Err(err) = store.put_crawl(record).await {
                tracing::error!(crawl_id = %crawl_id, error = %err, "failed to persist crawl result");
            }
            clear_active(active, &crawl_id).await;
        });
    }

    async fn clear_active(&self, crawl_id: &str) {
        clear_active(self.active_crawl.clone(), crawl_id).await;
    }
}

fn validate_request(request: CrawlRequest) -> Result<CrawlRequest, ServiceError> {
    let mut request = request.normalized();
    if request.seed_urls.is_empty() {
        return Err(ServiceError::InvalidRequest(
            "seed_urls must contain at least one URL".to_string(),
        ));
    }

    request.seed_urls = request
        .seed_urls
        .into_iter()
        .map(|url| normalize_url(&url).map_err(|err| ServiceError::InvalidRequest(err.to_string())))
        .collect::<Result<Vec<_>, _>>()?;
    request.seed_urls.sort();
    request.seed_urls.dedup();

    Ok(request)
}

async fn clear_active(active: Arc<Mutex<Option<String>>>, crawl_id: &str) {
    let mut active = active.lock().await;
    if active.as_deref() == Some(crawl_id) {
        *active = None;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::*;
    use crate::storage::InMemoryStorage;

    struct WaitingRunner {
        started: Arc<Notify>,
        finish: Arc<Notify>,
    }

    #[async_trait]
    impl CrawlRunner for WaitingRunner {
        async fn run(&self, _execution: CrawlExecution) -> anyhow::Result<CrawlStats> {
            self.started.notify_waiters();
            self.finish.notified().await;
            Ok(CrawlStats {
                pages_fetched: 1,
                pages_failed: 0,
            })
        }
    }

    #[tokio::test]
    async fn rejects_empty_seed_urls() {
        let store = Arc::new(InMemoryStorage::new());
        let runner = Arc::new(WaitingRunner {
            started: Arc::new(Notify::new()),
            finish: Arc::new(Notify::new()),
        });
        let service = CrawlerService::new(store, runner);

        let err = service
            .start_crawl(CrawlRequest {
                seed_urls: Vec::new(),
                max_pages: 10,
                depth_limit: 1,
            })
            .await
            .unwrap_err();

        assert!(matches!(err, ServiceError::InvalidRequest(_)));
    }

    #[tokio::test]
    async fn allows_only_one_active_crawl() {
        let store = Arc::new(InMemoryStorage::new());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let runner = Arc::new(WaitingRunner {
            started: started.clone(),
            finish: finish.clone(),
        });
        let service = CrawlerService::new(store, runner);

        service
            .start_crawl(CrawlRequest {
                seed_urls: vec!["https://example.com".to_string()],
                max_pages: 10,
                depth_limit: 1,
            })
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();

        let err = service
            .start_crawl(CrawlRequest {
                seed_urls: vec!["https://example.org".to_string()],
                max_pages: 10,
                depth_limit: 1,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, ServiceError::Busy(_)));

        finish.notify_waiters();
    }
}
