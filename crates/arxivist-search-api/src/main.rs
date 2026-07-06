use anyhow::{Context, Result};
use arxivist_core::{
    DocumentShard, PostingsShard, RankedResult, SearchIndex, ShardedDocument, ShardedIndexManifest,
    ShardedTermStats, bm25, document_shard_for_id, snippet, tfidf, tokenize,
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, Method},
    routing::{get, post},
};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, default_value = "data/dev/index")]
    index: PathBuf,
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, default_value_t = 32, env = "ARXIVIST_POSTINGS_CACHE_SHARDS")]
    postings_cache_shards: usize,
    #[arg(long, default_value_t = 64, env = "ARXIVIST_DOC_CACHE_SHARDS")]
    doc_cache_shards: usize,
}

#[derive(Clone)]
struct AppState {
    index: Arc<LoadedIndex>,
}

enum LoadedIndex {
    Legacy(SearchIndex),
    Sharded(ShardedIndex),
}

struct ShardedIndex {
    manifest: ShardedIndexManifest,
    terms: HashMap<String, ShardedTermStats>,
    store: ShardedIndexStore,
    postings_cache: tokio::sync::Mutex<ShardCache<PostingsShard>>,
    doc_cache: tokio::sync::Mutex<ShardCache<DocumentShard>>,
}

#[derive(Clone)]
enum ShardedIndexStore {
    Local { root: PathBuf },
}

struct ShardCache<T> {
    limit: usize,
    values: HashMap<usize, Arc<T>>,
    order: Vec<usize>,
}

#[derive(Debug, Deserialize)]
struct SearchRequest {
    query: String,
    #[serde(default)]
    top_k: Option<isize>,
    #[serde(default)]
    page: Option<isize>,
    #[serde(default)]
    page_size: Option<isize>,
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
    page: usize,
    page_size: usize,
    total_results: usize,
    total_pages: usize,
    has_previous: bool,
    has_next: bool,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    documents: usize,
    terms: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    index_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    postings_cache_shards: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    doc_cache_shards: Option<usize>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .compact()
        .init();

    let args = Args::parse();
    let index = load_index(&args).await?;
    let state = AppState {
        index: Arc::new(index),
    };

    let app = Router::new()
        .route("/health", get(health)) //is our api end point alive
        .route("/search", post(search)) // SEARCH FOR THE PAGES
        .with_state(state)
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http());

    info!(bind = %args.bind, "starting search api");
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn load_index(args: &Args) -> Result<LoadedIndex> {
    if args.index.is_dir() {
        let manifest_path = args.index.join("manifest.json");
        let manifest: ShardedIndexManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).with_context(|| {
                format!("read local sharded manifest {}", manifest_path.display())
            })?)
            .context("decode sharded index manifest")?;
        let terms_path = args.index.join(&manifest.terms_path);
        let terms = serde_json::from_slice(
            &std::fs::read(&terms_path)
                .with_context(|| format!("read local sharded terms {}", terms_path.display()))?,
        )
        .context("decode sharded index terms")?;
        return Ok(LoadedIndex::Sharded(ShardedIndex::new(
            manifest,
            terms,
            ShardedIndexStore::Local {
                root: args.index.clone(),
            },
            args.postings_cache_shards,
            args.doc_cache_shards,
        )));
    }

    let bytes = std::fs::read(&args.index)
        .with_context(|| format!("read local index {}", args.index.display()))?;
    let mut index: SearchIndex = serde_json::from_slice(&bytes).context("decode search index")?;
    index.ensure_inverted_index();
    Ok(LoadedIndex::Legacy(index))
}

impl ShardedIndex {
    fn new(
        manifest: ShardedIndexManifest,
        terms: HashMap<String, ShardedTermStats>,
        store: ShardedIndexStore,
        postings_cache_shards: usize,
        doc_cache_shards: usize,
    ) -> Self {
        Self {
            manifest,
            terms,
            store,
            postings_cache: tokio::sync::Mutex::new(ShardCache::new(postings_cache_shards)),
            doc_cache: tokio::sync::Mutex::new(ShardCache::new(doc_cache_shards)),
        }
    }

    async fn postings_shard(&self, shard: usize) -> Result<Arc<PostingsShard>> {
        if let Some(value) = self.postings_cache.lock().await.get(shard) {
            return Ok(value);
        }

        let path = format!("postings/{shard}.json");
        let shard_value: PostingsShard = serde_json::from_slice(&self.store.read(&path).await?)
            .with_context(|| format!("decode postings shard {shard}"))?;
        let shard_value = Arc::new(shard_value);
        self.postings_cache
            .lock()
            .await
            .insert(shard, shard_value.clone());
        Ok(shard_value)
    }

    async fn doc_shard(&self, shard: usize) -> Result<Arc<DocumentShard>> {
        if let Some(value) = self.doc_cache.lock().await.get(shard) {
            return Ok(value);
        }

        let path = format!("docs/{shard}.json");
        let shard_value: DocumentShard = serde_json::from_slice(&self.store.read(&path).await?)
            .with_context(|| format!("decode document shard {shard}"))?;
        let shard_value = Arc::new(shard_value);
        self.doc_cache
            .lock()
            .await
            .insert(shard, shard_value.clone());
        Ok(shard_value)
    }

    async fn document(&self, document_id: usize) -> Option<ShardedDocument> {
        let shard = document_shard_for_id(document_id, self.manifest.doc_shard_size);
        let docs = self.doc_shard(shard).await.ok()?;
        docs.documents
            .iter()
            .find(|document| document.id == document_id)
            .cloned()
    }
}

impl ShardedIndexStore {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        match self {
            ShardedIndexStore::Local { root } => {
                let path = root.join(path);
                std::fs::read(&path).with_context(|| {
                    format!("read local sharded index artifact {}", path.display())
                })
            }
        }
    }
}

impl<T> ShardCache<T> {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            values: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn get(&mut self, shard: usize) -> Option<Arc<T>> {
        self.values.get(&shard).cloned()
    }

    fn insert(&mut self, shard: usize, value: Arc<T>) {
        if self.limit == 0 {
            return;
        }

        if !self.values.contains_key(&shard) {
            self.order.push(shard);
        }
        self.values.insert(shard, value);

        while self.values.len() > self.limit {
            if self.order.is_empty() {
                break;
            }
            let evicted = self.order.remove(0);
            self.values.remove(&evicted);
        }
    }

    fn len(&self) -> usize {
        self.values.len()
    }
}

async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let (documents, terms, index_version, postings_cache_shards, doc_cache_shards) = match state
        .index
        .as_ref()
    {
        LoadedIndex::Legacy(index) => (index.documents.len(), index.terms.len(), None, None, None),
        LoadedIndex::Sharded(index) => (
            index.manifest.document_count,
            index.terms.len(),
            Some(index.manifest.version.clone()),
            Some(index.postings_cache.lock().await.len()),
            Some(index.doc_cache.lock().await.len()),
        ),
    };
    Json(HealthResponse {
        status: "ok",
        documents,
        terms,
        index_version,
        postings_cache_shards,
        doc_cache_shards,
    })
}

async fn search(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Json<SearchResponse> {
    Json(search_loaded_index(&state.index, request).await)
}

async fn search_loaded_index(index: &LoadedIndex, request: SearchRequest) -> SearchResponse {
    match index {
        LoadedIndex::Legacy(index) => search_index(index, request),
        LoadedIndex::Sharded(index) => search_sharded_index(index, request).await,
    }
}

async fn search_sharded_index(index: &ShardedIndex, request: SearchRequest) -> SearchResponse {
    let terms = tokenize(&request.query);
    let index_terms = matching_sharded_index_terms(index, &terms);
    let mut candidate_scores: HashMap<usize, CandidateScore> = HashMap::new();

    for (query_term, index_term) in &index_terms {
        let Some(stats) = index.terms.get(index_term) else {
            continue;
        };
        let Ok(postings_shard) = index.postings_shard(stats.postings_shard).await else {
            continue;
        };
        let Some(postings) = postings_shard.postings.get(index_term) else {
            continue;
        };
        let idf =
            inverse_document_frequency(index.manifest.document_count, stats.document_frequency);

        for posting in postings {
            let Some(doc) = index.document(posting.document_id).await else {
                continue;
            };
            let body_frequency = posting.body_frequency();

            let bm25_score = bm25(
                body_frequency,
                doc.token_count,
                index.manifest.average_doc_len,
                index.manifest.document_count,
                stats.document_frequency,
            );
            let tfidf_score = tfidf(
                body_frequency,
                doc.token_count,
                index.manifest.document_count,
                stats.document_frequency,
            );
            let title_score = posting.title_frequency as f64 * idf * 3.0;
            let url_score = posting.url_frequency as f64 * idf * 1.5;
            let score = candidate_scores
                .entry(posting.document_id)
                .or_insert_with(CandidateScore::default);
            score.bm25 += bm25_score;
            score.tfidf += tfidf_score;
            score.field_boost += title_score + url_score;
            score.matched_query_terms.insert(query_term.clone());
            score
                .body_positions_by_query_term
                .entry(query_term.clone())
                .or_default()
                .extend(posting.body_positions.iter().copied());
        }
    }

    let mut results = Vec::with_capacity(candidate_scores.len());
    for (document_id, score_parts) in candidate_scores {
        let Some(doc) = index.document(document_id).await else {
            continue;
        };

        let phrase_boost =
            exact_phrase_boost_for_fields(&request.query, doc.title.as_deref(), &doc.text);
        let proximity_boost = proximity_boost(&terms, &score_parts.body_positions_by_query_term);
        let coordination_boost =
            1.0 + (score_parts.matched_query_terms.len().saturating_sub(1) as f64 * 0.12);
        let text_score = (score_parts.bm25
            + score_parts.tfidf
            + score_parts.field_boost
            + phrase_boost
            + proximity_boost)
            * coordination_boost;
        let score = text_score * doc.page_rank.max(0.1);
        if score > 0.0 {
            results.push(RankedResult {
                url: doc.url,
                title: doc.title,
                snippet: snippet(&doc.text, &terms),
                score,
                bm25_score: score_parts.bm25,
                tfidf_score: score_parts.tfidf,
                page_rank: doc.page_rank,
            });
        }
    }

    finish_response(request, results)
}

fn search_index(index: &SearchIndex, request: SearchRequest) -> SearchResponse {
    let terms = tokenize(&request.query);
    let index_terms = matching_index_terms(index, &terms);
    let mut candidate_scores: HashMap<usize, CandidateScore> = HashMap::new();

    // The inverted index gives us only documents containing at least one query
    // term, so ranking does not need to scan every stored document.
    for (query_term, index_term) in &index_terms {
        let Some(stats) = index.terms.get(index_term) else {
            continue;
        };
        let Some(postings) = index.inverted_index.get(index_term) else {
            continue;
        };
        let idf = inverse_document_frequency(index.documents.len(), stats.document_frequency);

        for posting in postings {
            let Some(doc) = index.documents.get(posting.document_id) else {
                continue;
            };
            let body_frequency = posting.body_frequency();

            let bm25_score = bm25(
                body_frequency,
                doc.token_count,
                index.average_doc_len,
                index.documents.len(),
                stats.document_frequency,
            );
            let tfidf_score = tfidf(
                body_frequency,
                doc.token_count,
                index.documents.len(),
                stats.document_frequency,
            );
            let title_score = posting.title_frequency as f64 * idf * 3.0;
            let url_score = posting.url_frequency as f64 * idf * 1.5;
            let score = candidate_scores
                .entry(posting.document_id)
                .or_insert_with(CandidateScore::default);
            score.bm25 += bm25_score;
            score.tfidf += tfidf_score;
            score.field_boost += title_score + url_score;
            score.matched_query_terms.insert(query_term.clone());
            score
                .body_positions_by_query_term
                .entry(query_term.clone())
                .or_default()
                .extend(posting.body_positions.iter().copied());
        }
    }

    let mut results = Vec::with_capacity(candidate_scores.len());
    for (document_id, score_parts) in candidate_scores {
        let Some(doc) = index.documents.get(document_id) else {
            continue;
        };

        let phrase_boost = exact_phrase_boost(&request.query, doc);
        let proximity_boost = proximity_boost(&terms, &score_parts.body_positions_by_query_term);
        let coordination_boost =
            1.0 + (score_parts.matched_query_terms.len().saturating_sub(1) as f64 * 0.12);
        let text_score = (score_parts.bm25
            + score_parts.tfidf
            + score_parts.field_boost
            + phrase_boost
            + proximity_boost)
            * coordination_boost;
        let score = text_score * doc.page_rank.max(0.1);
        if score > 0.0 {
            results.push(RankedResult {
                url: doc.url.clone(),
                title: doc.title.clone(),
                snippet: snippet(&doc.text, &terms),
                score,
                bm25_score: score_parts.bm25,
                tfidf_score: score_parts.tfidf,
                page_rank: doc.page_rank,
            });
        }
    }

    finish_response(request, results)
}

fn finish_response(request: SearchRequest, mut results: Vec<RankedResult>) -> SearchResponse {
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let pagination = Pagination::from_request(&request);
    let total_results = results.len();
    let total_pages = if total_results == 0 {
        0
    } else {
        total_results.div_ceil(pagination.page_size)
    };
    let start = (pagination.page - 1) * pagination.page_size;
    let page_results = results
        .into_iter()
        .skip(start)
        .take(pagination.page_size)
        .collect();

    SearchResponse {
        query: request.query,
        mode: match request.mode {
            SearchMode::Traditional => "traditional".to_owned(),
        },
        results: page_results,
        page: pagination.page,
        page_size: pagination.page_size,
        total_results,
        total_pages,
        has_previous: total_pages > 0 && pagination.page > 1,
        has_next: pagination.page < total_pages,
    }
}

#[derive(Default)]
struct CandidateScore {
    bm25: f64,
    tfidf: f64,
    field_boost: f64,
    matched_query_terms: HashSet<String>,
    body_positions_by_query_term: HashMap<String, Vec<u32>>,
}

fn proximity_boost(
    query_terms: &[String],
    positions_by_query_term: &HashMap<String, Vec<u32>>,
) -> f64 {
    let distinct_terms = distinct_query_terms(query_terms);
    if distinct_terms.len() < 2 {
        return 0.0;
    }

    let mut positioned_terms = Vec::with_capacity(distinct_terms.len());
    for term in &distinct_terms {
        let Some(positions) = positions_by_query_term.get(term) else {
            return 0.0;
        };
        if positions.is_empty() {
            return 0.0;
        }
        positioned_terms.push((term, positions));
    }

    let mut positioned_tokens = Vec::new();
    for (term_index, (_, positions)) in positioned_terms.iter().enumerate() {
        for position in positions.iter().copied() {
            positioned_tokens.push((position, term_index));
        }
    }
    positioned_tokens.sort_unstable_by_key(|(position, _)| *position);

    let mut counts = vec![0usize; distinct_terms.len()];
    let mut covered = 0usize;
    let mut left = 0usize;
    let mut best_span: Option<u32> = None;

    for right in 0..positioned_tokens.len() {
        let (_, term_index) = positioned_tokens[right];
        if counts[term_index] == 0 {
            covered += 1;
        }
        counts[term_index] += 1;

        while covered == distinct_terms.len() {
            let span = positioned_tokens[right].0 - positioned_tokens[left].0 + 1;
            best_span = Some(best_span.map_or(span, |best| best.min(span)));

            let (_, left_term_index) = positioned_tokens[left];
            counts[left_term_index] -= 1;
            if counts[left_term_index] == 0 {
                covered -= 1;
            }
            left += 1;
        }
    }

    match best_span {
        Some(span) if span as usize == distinct_terms.len() => 8.0,
        Some(span) if span <= 8 => 5.0,
        Some(span) if span <= 20 => 2.0,
        _ => 0.0,
    }
}

fn distinct_query_terms(query_terms: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    query_terms
        .iter()
        .filter(|term| seen.insert((*term).clone()))
        .cloned()
        .collect()
}

fn matching_index_terms(index: &SearchIndex, query_terms: &[String]) -> Vec<(String, String)> {
    let mut matches = Vec::new();
    for term in query_terms {
        if index.inverted_index.contains_key(term) {
            matches.push((term.clone(), term.clone()));
        }
    }

    if !matches.is_empty() {
        return matches;
    }

    let mut seen = HashSet::new();
    for term in query_terms {
        if term.len() < 3 {
            continue;
        }

        let mut expanded_terms: Vec<_> = index
            .inverted_index
            .keys()
            .filter(|candidate| candidate.starts_with(term))
            .cloned()
            .collect();
        expanded_terms.sort();

        for expanded_term in expanded_terms.into_iter().take(8) {
            if seen.insert((term.clone(), expanded_term.clone())) {
                matches.push((term.clone(), expanded_term));
            }
        }
    }

    matches
}

fn matching_sharded_index_terms(
    index: &ShardedIndex,
    query_terms: &[String],
) -> Vec<(String, String)> {
    let mut matches = Vec::new();
    for term in query_terms {
        if index.terms.contains_key(term) {
            matches.push((term.clone(), term.clone()));
        }
    }

    if !matches.is_empty() {
        return matches;
    }

    let mut seen = HashSet::new();
    for term in query_terms {
        if term.len() < 3 {
            continue;
        }

        let mut expanded_terms: Vec<_> = index
            .terms
            .keys()
            .filter(|candidate| candidate.starts_with(term))
            .cloned()
            .collect();
        expanded_terms.sort();

        for expanded_term in expanded_terms.into_iter().take(8) {
            if seen.insert((term.clone(), expanded_term.clone())) {
                matches.push((term.clone(), expanded_term));
            }
        }
    }

    matches
}

fn inverse_document_frequency(total_docs: usize, doc_freq: usize) -> f64 {
    if total_docs == 0 || doc_freq == 0 {
        return 0.0;
    }

    ((total_docs as f64 + 1.0) / (doc_freq as f64 + 1.0)).ln() + 1.0
}

fn exact_phrase_boost(query: &str, doc: &arxivist_core::IndexedDocument) -> f64 {
    exact_phrase_boost_for_fields(query, doc.title.as_deref(), &doc.text)
}

fn exact_phrase_boost_for_fields(query: &str, title: Option<&str>, text: &str) -> f64 {
    let normalized_query = query.trim().to_ascii_lowercase();
    if normalized_query.len() < 3 {
        return 0.0;
    }

    let mut boost = 0.0;
    if title.is_some_and(|title| title.to_ascii_lowercase().contains(&normalized_query)) {
        boost += 4.0;
    }
    if text.to_ascii_lowercase().contains(&normalized_query) {
        boost += 1.5;
    }

    boost
}

struct Pagination {
    page: usize,
    page_size: usize,
}

impl Pagination {
    fn from_request(request: &SearchRequest) -> Self {
        // `top_k` remains accepted for old clients, while new callers should
        // send page/page_size so result windows are explicit.
        let page_size = request
            .page_size
            .or(request.top_k)
            .unwrap_or(10)
            .clamp(1, 50) as usize;
        let page = request.page.unwrap_or(1).max(1) as usize;

        Self { page, page_size }
    }
}

//cross origin resource sharing. tldr for running apis locally. never heard of this before.
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

#[cfg(test)]
mod tests {
    use super::*;
    use arxivist_core::{IndexedDocument, Posting, TermStats};
    use std::collections::HashMap;

    #[test]
    fn default_search_returns_first_page_metadata() {
        let response = search_index(&test_index(12), request("rust", None, None, None));

        assert_eq!(response.page, 1);
        assert_eq!(response.page_size, 10);
        assert_eq!(response.total_results, 12);
        assert_eq!(response.total_pages, 2);
        assert!(!response.has_previous);
        assert!(response.has_next);
        assert_eq!(response.results.len(), 10);
    }

    #[test]
    fn page_two_returns_next_slice_without_repeating_page_one() {
        let page_one = search_index(&test_index(12), request("rust", None, Some(1), Some(10)));
        let page_two = search_index(&test_index(12), request("rust", None, Some(2), Some(10)));

        let first_page_urls: Vec<_> = page_one
            .results
            .iter()
            .map(|result| result.url.as_str().to_owned())
            .collect();

        assert_eq!(page_two.page, 2);
        assert_eq!(page_two.results.len(), 2);
        assert!(page_two.has_previous);
        assert!(!page_two.has_next);
        assert!(
            page_two
                .results
                .iter()
                .all(|result| !first_page_urls.contains(&result.url.as_str().to_owned()))
        );
    }

    #[test]
    fn no_match_returns_empty_pagination_metadata() {
        let response = search_index(&test_index(12), request("python", None, None, None));

        assert_eq!(response.results.len(), 0);
        assert_eq!(response.total_results, 0);
        assert_eq!(response.total_pages, 0);
        assert!(!response.has_previous);
        assert!(!response.has_next);
    }

    #[test]
    fn oversized_page_size_clamps_to_fifty() {
        let response = search_index(&test_index(60), request("rust", None, Some(1), Some(500)));

        assert_eq!(response.page_size, 50);
        assert_eq!(response.results.len(), 50);
        assert_eq!(response.total_results, 60);
        assert_eq!(response.total_pages, 2);
    }

    #[test]
    fn invalid_page_values_clamp_to_minimums() {
        let response = search_index(&test_index(12), request("rust", None, Some(-4), Some(-2)));

        assert_eq!(response.page, 1);
        assert_eq!(response.page_size, 1);
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.total_pages, 12);
    }

    #[test]
    fn top_k_request_remains_accepted() {
        let response = search_index(&test_index(12), request("rust", Some(5), None, None));

        assert_eq!(response.page, 1);
        assert_eq!(response.page_size, 5);
        assert_eq!(response.results.len(), 5);
        assert_eq!(response.total_results, 12);
    }

    #[test]
    fn search_uses_inverted_index_for_candidates() {
        let mut index = test_index(3);
        for doc in &mut index.documents {
            doc.term_freqs.clear();
        }

        let response = search_index(&index, request("rust", None, None, None));

        assert_eq!(response.total_results, 3);
        assert_eq!(response.results.len(), 3);
    }

    #[test]
    fn multi_term_search_unions_posting_candidates() {
        let index = two_term_index();
        let response = search_index(&index, request("rust python", None, None, None));
        let urls: Vec<_> = response
            .results
            .iter()
            .map(|result| result.url.as_str().to_owned())
            .collect();

        assert_eq!(response.total_results, 2);
        assert!(urls.contains(&"https://example.com/rust".to_owned()));
        assert!(urls.contains(&"https://example.com/python".to_owned()));
    }

    #[test]
    fn old_index_without_postings_can_be_repaired() {
        let old_index_json = r#"{
            "documents": [{
                "id": 0,
                "url": "https://example.com/rust",
                "title": "Rust",
                "text": "Rust search document",
                "token_count": 3,
                "term_freqs": { "rust": 1 },
                "page_rank": 1.0
            }],
            "terms": { "rust": { "document_frequency": 1 } },
            "average_doc_len": 3.0
        }"#;
        let mut index: SearchIndex = serde_json::from_str(old_index_json).unwrap();

        assert!(index.inverted_index.is_empty());
        index.ensure_inverted_index();

        let response = search_index(&index, request("rust", None, None, None));
        assert_eq!(response.total_results, 1);
    }

    #[test]
    fn title_match_ranks_above_body_only_match() {
        let index = SearchIndex {
            documents: vec![
                document(
                    0,
                    "https://example.com/body",
                    Some("General Notes"),
                    "rust appears once in a long generic document with many filler words",
                    HashMap::from([("rust".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
                document(
                    1,
                    "https://example.com/title",
                    Some("Rust Guide"),
                    "general notes with enough text for a search result snippet",
                    HashMap::new(),
                    HashMap::from([("rust".to_owned(), 1)]),
                    HashMap::new(),
                ),
            ],
            terms: HashMap::from([(
                "rust".to_owned(),
                TermStats {
                    document_frequency: 2,
                },
            )]),
            inverted_index: HashMap::from([(
                "rust".to_owned(),
                vec![posting(0, 1, 0, 0), posting(1, 0, 1, 0)],
            )]),
            average_doc_len: 11.0,
        };

        let response = search_index(&index, request("rust", None, None, None));

        assert_eq!(response.results[0].title.as_deref(), Some("Rust Guide"));
    }

    #[test]
    fn url_match_returns_relevant_result() {
        let index = SearchIndex {
            documents: vec![document(
                0,
                "https://example.com/topics/neural-retrieval",
                Some("General Topic"),
                "general notes with enough text for a search result snippet",
                HashMap::new(),
                HashMap::new(),
                HashMap::from([("neural".to_owned(), 1)]),
            )],
            terms: HashMap::from([(
                "neural".to_owned(),
                TermStats {
                    document_frequency: 1,
                },
            )]),
            inverted_index: HashMap::from([("neural".to_owned(), vec![posting(0, 0, 0, 1)])]),
            average_doc_len: 9.0,
        };

        let response = search_index(&index, request("neural", None, None, None));

        assert_eq!(response.total_results, 1);
        assert_eq!(
            response.results[0].url.as_str(),
            "https://example.com/topics/neural-retrieval"
        );
    }

    #[test]
    fn multi_term_query_boosts_documents_matching_more_terms() {
        let index = SearchIndex {
            documents: vec![
                document(
                    0,
                    "https://example.com/rust-python",
                    Some("Rust Python"),
                    "rust python bridge",
                    HashMap::from([("rust".to_owned(), 1), ("python".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
                document(
                    1,
                    "https://example.com/rust",
                    Some("Rust"),
                    "rust language notes",
                    HashMap::from([("rust".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
            ],
            terms: HashMap::from([
                (
                    "rust".to_owned(),
                    TermStats {
                        document_frequency: 2,
                    },
                ),
                (
                    "python".to_owned(),
                    TermStats {
                        document_frequency: 1,
                    },
                ),
            ]),
            inverted_index: HashMap::from([
                (
                    "rust".to_owned(),
                    vec![posting(0, 1, 0, 0), posting(1, 1, 0, 0)],
                ),
                ("python".to_owned(), vec![posting(0, 1, 0, 0)]),
            ]),
            average_doc_len: 3.0,
        };

        let response = search_index(&index, request("rust python", None, None, None));

        assert_eq!(
            response.results[0].url.as_str(),
            "https://example.com/rust-python"
        );
    }

    #[test]
    fn exact_phrase_in_title_boosts_ranking() {
        let index = SearchIndex {
            documents: vec![
                document(
                    0,
                    "https://example.com/phrase-title",
                    Some("Rust Search"),
                    "general notes with enough text for a search result snippet",
                    HashMap::new(),
                    HashMap::from([("rust".to_owned(), 1), ("search".to_owned(), 1)]),
                    HashMap::new(),
                ),
                document(
                    1,
                    "https://example.com/body",
                    Some("Body Match"),
                    "rust and search both appear here with enough content",
                    HashMap::from([("rust".to_owned(), 1), ("search".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
            ],
            terms: HashMap::from([
                (
                    "rust".to_owned(),
                    TermStats {
                        document_frequency: 2,
                    },
                ),
                (
                    "search".to_owned(),
                    TermStats {
                        document_frequency: 2,
                    },
                ),
            ]),
            inverted_index: HashMap::from([
                (
                    "rust".to_owned(),
                    vec![posting(0, 0, 1, 0), posting(1, 1, 0, 0)],
                ),
                (
                    "search".to_owned(),
                    vec![posting(0, 0, 1, 0), posting(1, 1, 0, 0)],
                ),
            ]),
            average_doc_len: 8.0,
        };

        let response = search_index(&index, request("rust search", None, None, None));

        assert_eq!(response.results[0].title.as_deref(), Some("Rust Search"));
    }

    #[test]
    fn nearby_body_terms_rank_above_far_apart_terms() {
        let index = SearchIndex {
            documents: vec![
                document(
                    0,
                    "https://example.com/far",
                    Some("Far Terms"),
                    "formula filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler filler 1",
                    HashMap::from([("formula".to_owned(), 1), ("1".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
                document(
                    1,
                    "https://example.com/near",
                    Some("Near Terms"),
                    "formula 1 racing",
                    HashMap::from([("formula".to_owned(), 1), ("1".to_owned(), 1)]),
                    HashMap::new(),
                    HashMap::new(),
                ),
            ],
            terms: HashMap::from([
                (
                    "formula".to_owned(),
                    TermStats {
                        document_frequency: 2,
                    },
                ),
                (
                    "1".to_owned(),
                    TermStats {
                        document_frequency: 2,
                    },
                ),
            ]),
            inverted_index: HashMap::from([
                (
                    "formula".to_owned(),
                    vec![
                        posting_with_positions(0, 1, 0, 0, vec![0]),
                        posting_with_positions(1, 1, 0, 0, vec![0]),
                    ],
                ),
                (
                    "1".to_owned(),
                    vec![
                        posting_with_positions(0, 1, 0, 0, vec![20]),
                        posting_with_positions(1, 1, 0, 0, vec![1]),
                    ],
                ),
            ]),
            average_doc_len: 12.0,
        };

        let response = search_index(&index, request("formula 1", None, None, None));

        assert_eq!(response.results[0].url.as_str(), "https://example.com/near");
    }

    #[test]
    fn exact_adjacent_terms_boost_more_than_broad_proximity() {
        let exact = HashMap::from([("formula".to_owned(), vec![4]), ("1".to_owned(), vec![5])]);
        let nearby = HashMap::from([("formula".to_owned(), vec![4]), ("1".to_owned(), vec![11])]);

        assert!(
            proximity_boost(&tokenize("formula 1"), &exact)
                > proximity_boost(&tokenize("formula 1"), &nearby)
        );
    }

    #[test]
    fn prefix_fallback_returns_results_when_exact_terms_do_not_match() {
        let index = SearchIndex {
            documents: vec![document(
                0,
                "https://example.com/neuralnetwork",
                Some("Neuralnetwork Overview"),
                "general notes with enough text for a search result snippet",
                HashMap::new(),
                HashMap::from([("neuralnetwork".to_owned(), 1)]),
                HashMap::new(),
            )],
            terms: HashMap::from([(
                "neuralnetwork".to_owned(),
                TermStats {
                    document_frequency: 1,
                },
            )]),
            inverted_index: HashMap::from([(
                "neuralnetwork".to_owned(),
                vec![posting(0, 0, 1, 0)],
            )]),
            average_doc_len: 9.0,
        };

        let response = search_index(&index, request("neural", None, None, None));

        assert_eq!(response.total_results, 1);
        assert_eq!(
            response.results[0].title.as_deref(),
            Some("Neuralnetwork Overview")
        );
    }

    fn request(
        query: &str,
        top_k: Option<isize>,
        page: Option<isize>,
        page_size: Option<isize>,
    ) -> SearchRequest {
        SearchRequest {
            query: query.to_owned(),
            top_k,
            page,
            page_size,
            mode: SearchMode::Traditional,
        }
    }

    fn test_index(document_count: usize) -> SearchIndex {
        let documents: Vec<_> = (0..document_count)
            .map(|id| {
                let mut term_freqs = HashMap::new();
                term_freqs.insert("rust".to_owned(), 1);

                IndexedDocument {
                    id,
                    url: format!("https://example.com/{id}").parse().unwrap(),
                    title: Some(format!("Document {id}")),
                    text: format!("Rust search document {id}"),
                    token_count: 4,
                    term_freqs,
                    title_term_freqs: HashMap::new(),
                    url_term_freqs: HashMap::new(),
                    page_rank: document_count.saturating_sub(id) as f64 + 1.0,
                }
            })
            .collect();
        let inverted_index = HashMap::from([(
            "rust".to_owned(),
            (0..document_count)
                .map(|document_id| posting(document_id, 1, 0, 0))
                .collect(),
        )]);

        SearchIndex {
            documents,
            terms: HashMap::from([(
                "rust".to_owned(),
                TermStats {
                    document_frequency: document_count,
                },
            )]),
            inverted_index,
            average_doc_len: 4.0,
        }
    }

    fn two_term_index() -> SearchIndex {
        let documents = vec![
            IndexedDocument {
                id: 0,
                url: "https://example.com/rust".parse().unwrap(),
                title: Some("Rust".to_owned()),
                text: "Rust search".to_owned(),
                token_count: 2,
                term_freqs: HashMap::from([("rust".to_owned(), 1)]),
                title_term_freqs: HashMap::new(),
                url_term_freqs: HashMap::new(),
                page_rank: 1.0,
            },
            IndexedDocument {
                id: 1,
                url: "https://example.com/python".parse().unwrap(),
                title: Some("Python".to_owned()),
                text: "Python search".to_owned(),
                token_count: 2,
                term_freqs: HashMap::from([("python".to_owned(), 1)]),
                title_term_freqs: HashMap::new(),
                url_term_freqs: HashMap::new(),
                page_rank: 1.0,
            },
        ];

        SearchIndex {
            documents,
            terms: HashMap::from([
                (
                    "rust".to_owned(),
                    TermStats {
                        document_frequency: 1,
                    },
                ),
                (
                    "python".to_owned(),
                    TermStats {
                        document_frequency: 1,
                    },
                ),
            ]),
            inverted_index: HashMap::from([
                ("rust".to_owned(), vec![posting(0, 1, 0, 0)]),
                ("python".to_owned(), vec![posting(1, 1, 0, 0)]),
            ]),
            average_doc_len: 2.0,
        }
    }

    fn posting(
        document_id: usize,
        body_frequency: usize,
        title_frequency: usize,
        url_frequency: usize,
    ) -> Posting {
        posting_with_positions(
            document_id,
            body_frequency,
            title_frequency,
            url_frequency,
            Vec::new(),
        )
    }

    fn posting_with_positions(
        document_id: usize,
        body_frequency: usize,
        title_frequency: usize,
        url_frequency: usize,
        body_positions: Vec<u32>,
    ) -> Posting {
        Posting {
            document_id,
            term_frequency: body_frequency,
            body_frequency,
            title_frequency,
            url_frequency,
            body_positions,
        }
    }

    fn document(
        id: usize,
        url: &str,
        title: Option<&str>,
        text: &str,
        term_freqs: HashMap<String, usize>,
        title_term_freqs: HashMap<String, usize>,
        url_term_freqs: HashMap<String, usize>,
    ) -> IndexedDocument {
        let token_count = term_freqs.values().sum();

        IndexedDocument {
            id,
            url: url.parse().unwrap(),
            title: title.map(str::to_owned),
            text: text.to_owned(),
            token_count,
            term_freqs,
            title_term_freqs,
            url_term_freqs,
            page_rank: 1.0,
        }
    }
}
