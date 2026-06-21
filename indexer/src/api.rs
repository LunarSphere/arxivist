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
    models::{IndexBuildRecord, IndexRequest, IndexStartedResponse},
    service::{IndexerService, ServiceError},
};

#[derive(Clone)]
pub struct AppState {
    service: Arc<IndexerService>,
}

pub fn router(service: Arc<IndexerService>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/index", post(start_index))
        .route("/index/{index_build_id}", get(get_index))
        .with_state(AppState { service })
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn start_index(
    State(state): State<AppState>,
    Json(request): Json<IndexRequest>,
) -> Result<(StatusCode, Json<IndexStartedResponse>), ApiError> {
    let response = state.service.start_index(request).await?;
    Ok((StatusCode::ACCEPTED, Json(response)))
}

async fn get_index(
    State(state): State<AppState>,
    Path(index_build_id): Path<String>,
) -> Result<Json<IndexBuildRecord>, ApiError> {
    Ok(Json(state.service.get_index(&index_build_id).await?))
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

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
        models::IndexBuildStats,
        service::{IndexExecution, IndexRunner},
        storage::InMemoryIndexStorage,
    };

    struct BlockingRunner {
        started: Arc<Notify>,
        finish: Arc<Notify>,
    }

    #[async_trait]
    impl IndexRunner for BlockingRunner {
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
    async fn start_index_returns_accepted() {
        let app = test_router();
        let body = json!({ "crawl_id": "crawl-1" });

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/index")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn active_index_returns_conflict() {
        let store = Arc::new(InMemoryIndexStorage::new());
        let started = Arc::new(Notify::new());
        let finish = Arc::new(Notify::new());
        let runner = Arc::new(BlockingRunner {
            started: started.clone(),
            finish: finish.clone(),
        });
        let app = router(Arc::new(IndexerService::new(store, runner)));
        let body = json!({ "crawl_id": "crawl-1" });

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/index")
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
                    .uri("/index")
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
        let store = Arc::new(InMemoryIndexStorage::new());
        let runner = Arc::new(BlockingRunner {
            started: Arc::new(Notify::new()),
            finish: Arc::new(Notify::new()),
        });
        router(Arc::new(IndexerService::new(store, runner)))
    }
}
