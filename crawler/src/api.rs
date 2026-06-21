use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Serialize;

use crate::{
    models::{CrawlRecord, CrawlRequest, CrawlStartedResponse},
    service::{CrawlerService, ServiceError},
};

//are we crawling?
#[derive(Clone)]
pub struct AppState {
    service: Arc<CrawlerService>,
}

//define API requests
// get /health, post /crawl, get /crawl/{crawl_id}
pub fn router(service: Arc<CrawlerService>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/crawl", post(start_crawl))
        .route("/crawl/{crawl_id}", get(get_crawl))
        .with_state(AppState { service })
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}
//takes app state and request, and starts and async crawl.
async fn start_crawl(
    State(state): State<AppState>,
    Json(request): Json<CrawlRequest>,
) -> Result<(StatusCode, Json<CrawlStartedResponse>), ApiError> {
    let response = state.service.start_crawl(request).await?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}
// gets the health response of the crawl_id?
async fn get_crawl(
    State(state): State<AppState>,
    Path(crawl_id): Path<String>,
) -> Result<Json<CrawlRecord>, ApiError> {
    Ok(Json(state.service.get_crawl(&crawl_id).await?))
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

// storing and returning type of error
#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

pub struct ApiError(ServiceError);

impl From<ServiceError> for ApiError {
    fn from(value: ServiceError) -> Self {
        Self(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            ServiceError::InvalidRequest(_) => StatusCode::BAD_REQUEST,
            ServiceError::Busy(_) => StatusCode::CONFLICT,
            ServiceError::NotFound(_) => StatusCode::NOT_FOUND,
            ServiceError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use async_trait::async_trait;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use tokio::sync::Notify;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        models::CrawlStats,
        service::{CrawlExecution, CrawlRunner},
        storage::InMemoryStorage,
    };

    struct BlockingRunner {
        started: Arc<Notify>,
        finish: Arc<Notify>,
    }

    #[async_trait]
    impl CrawlRunner for BlockingRunner {
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
    async fn health_returns_ok() {
        let app = test_router();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn start_crawl_returns_accepted() {
        let app = test_router();
        let body = json!({
            "seed_urls": ["https://example.com"],
            "max_pages": 10,
            "depth_limit": 1
        });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/crawl")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn active_crawl_returns_conflict() {
        let store = Arc::new(InMemoryStorage::new());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let runner = Arc::new(BlockingRunner {
            started: started.clone(),
            finish: finish.clone(),
        });
        let app = router(Arc::new(CrawlerService::new(store, runner)));

        let body = json!({
            "seed_urls": ["https://example.com"],
            "max_pages": 10,
            "depth_limit": 1
        });

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/crawl")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);

        tokio::time::timeout(Duration::from_secs(1), started.notified())
            .await
            .unwrap();

        let second = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/crawl")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::CONFLICT);

        finish.notify_waiters();
    }

    fn test_router() -> Router {
        let store = Arc::new(InMemoryStorage::new());
        let runner = Arc::new(BlockingRunner {
            started: Arc::new(Notify::new()),
            finish: Arc::new(Notify::new()),
        });
        router(Arc::new(CrawlerService::new(store, runner)))
    }
}
