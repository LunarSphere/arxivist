use anyhow::Result;
use arxivist_core::{RankedResult, SearchIndex, bm25, snippet, tfidf, tokenize};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, Method},
    routing::{get, post},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{cmp::Ordering, net::SocketAddr, path::PathBuf, sync::Arc};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "data/dev/index/index.json")]
    index: PathBuf,
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
}

#[derive(Clone)]
struct AppState {
    index: Arc<SearchIndex>,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default = "default_top_k")]
    top_k: usize,
    #[serde(default)]
    mode: SearchMode,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum SearchMode {
    #[default]
    Traditional,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    mode: String,
    results: Vec<RankedResult>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    documents: usize,
    terms: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = Args::parse();
    let index: SearchIndex = serde_json::from_slice(&std::fs::read(&args.index)?)?;
    let state = AppState {
        index: Arc::new(index),
    };

    let app = Router::new()
        .route("/health", get(health))
        .route("/search", post(search))
        .with_state(state)
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http());

    info!(bind = %args.bind, "starting search api");
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        documents: state.index.documents.len(),
        terms: state.index.terms.len(),
    })
}

async fn search(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Json<SearchResponse> {
    let terms = tokenize(&request.query);
    let mut results = Vec::new();

    for doc in &state.index.documents {
        let mut bm25_score = 0.0;
        let mut tfidf_score = 0.0;

        for term in &terms {
            let Some(stats) = state.index.terms.get(term) else {
                continue;
            };
            let tf = doc.term_freqs.get(term).copied().unwrap_or(0);
            bm25_score += bm25(
                tf,
                doc.token_count,
                state.index.average_doc_len,
                state.index.documents.len(),
                stats.document_frequency,
            );
            tfidf_score += tfidf(
                tf,
                doc.token_count,
                state.index.documents.len(),
                stats.document_frequency,
            );
        }

        // PageRank is a quality multiplier, not a replacement for text relevance.
        let text_score = bm25_score + tfidf_score;
        let score = text_score * doc.page_rank.max(0.1);
        if score > 0.0 {
            results.push(RankedResult {
                url: doc.url.clone(),
                title: doc.title.clone(),
                snippet: snippet(&doc.text, &terms),
                score,
                bm25_score,
                tfidf_score,
                page_rank: doc.page_rank,
            });
        }
    }

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    results.truncate(request.top_k.min(50));

    Json(SearchResponse {
        query: request.query,
        mode: match request.mode {
            SearchMode::Traditional => "traditional".to_owned(),
        },
        results,
    })
}

fn default_top_k() -> usize {
    10
}

fn cors_layer() -> CorsLayer {
    let origin = std::env::var("ARXIVIST_CORS_ORIGIN").unwrap_or_else(|_| "*".to_owned());
    let layer = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([axum::http::header::CONTENT_TYPE]);

    if origin == "*" {
        layer.allow_origin(tower_http::cors::Any)
    } else {
        let origin = HeaderValue::from_str(&origin)
            .expect("ARXIVIST_CORS_ORIGIN must be a valid header value");
        layer.allow_origin(origin)
    }
}
