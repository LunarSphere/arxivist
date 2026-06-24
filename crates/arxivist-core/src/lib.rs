// define structs, enums, and functions reused in other crates
// an Enum represents a value that is one of servral possible variants
// TLDR structs: CrawlRecord, search document, indexed document, term stats, ranked result
// TLDR enums: Crawl Outocome, Crawl Skip Reason
// TLDR fn: content_hash, tokeninze, normaize_token, term_frequencies, bm25, tfidf, snippet
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use url::Url;

// Define structs and Enumns
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlRecord {
    #[serde(default = "crawl_schema_version")]
    // sets a default if the field is missing. | neato mosquito
    pub schema_version: u8,
    pub requested_url: Url,
    pub final_url: Option<Url>,
    pub source_seed: Url,
    pub referrer: Option<Url>,
    pub depth: usize,
    pub outcome: CrawlOutcome,
    pub skip_reason: Option<CrawlSkipReason>,
    pub title: Option<String>,
    pub status: Option<u16>,
    pub content_type: Option<String>,
    pub content_length: Option<u64>,
    pub content_hash: Option<String>,
    pub content_path: Option<String>,
    pub extracted_text: String,
    pub links: Vec<Url>,
    pub fetched_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrawlOutcome {
    Stored,
    Skipped,
    RobotsBlocked,
    HostSuppressed,
    FetchFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrawlSkipReason {
    RobotsTxt,
    NonHtml,
    EmptyText,
    LikelyJavascriptRequired,
    FetchError,
    BadHostThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchDocument {
    pub id: usize,
    pub url: Url,
    pub title: Option<String>,
    pub text: String,
    pub links: Vec<Url>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchIndex {
    pub documents: Vec<IndexedDocument>,
    pub terms: HashMap<String, TermStats>,
    pub average_doc_len: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedDocument {
    pub id: usize,
    pub url: Url,
    pub title: Option<String>,
    pub text: String,
    pub token_count: usize,
    pub term_freqs: HashMap<String, usize>,
    pub page_rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermStats {
    pub document_frequency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedResult {
    pub url: Url,
    pub title: Option<String>,
    pub snippet: String,
    pub score: f64,
    pub bm25_score: f64,
    pub tfidf_score: f64,
    pub page_rank: f64,
}

//private function
// private function to use with Crawl_record when crawl schema isnt specified.
fn crawl_schema_version() -> u8 {
    2
}

//PUBLIC FUNCTIONS
// creates a SHA256 hash based on the html body information
pub fn content_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

//splits string into a vector of strings
pub fn tokenize(input: &str) -> Vec<String> {
    input
        .split_whitespace()
        .map(normalize_token)
        .filter(|term| !term.is_empty())
        .filter(|term| !STOP_WORDS.contains(&term.as_str()))
        .collect()
}

// makes strings lowercase and drops non alphanumeric characters
pub fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// Hashmap with a term and how many times it appears
pub fn term_frequencies(tokens: &[String]) -> HashMap<String, usize> {
    let mut freqs = HashMap::new();
    for token in tokens {
        *freqs.entry(token.clone()).or_insert(0) += 1;
    }
    freqs
}

// BM25 rewards repeated terms but dampens very long documents | another document relevance metric
pub fn bm25(
    tf: usize,
    doc_len: usize,
    avg_doc_len: f64,
    total_docs: usize,
    doc_freq: usize,
) -> f64 {
    if tf == 0 || doc_len == 0 || total_docs == 0 || doc_freq == 0 {
        return 0.0;
    }
    let k1 = 1.2;
    let b = 0.75;
    let tf = tf as f64;
    let idf = (((total_docs as f64 - doc_freq as f64 + 0.5) / (doc_freq as f64 + 0.5)) + 1.0).ln();
    let length_norm = 1.0 - b + b * (doc_len as f64 / avg_doc_len.max(1.0));
    idf * ((tf * (k1 + 1.0)) / (tf + k1 * length_norm))
}
// term frequence inverse document frequencey | another document relevance metric
pub fn tfidf(tf: usize, doc_len: usize, total_docs: usize, doc_freq: usize) -> f64 {
    if tf == 0 || doc_len == 0 || total_docs == 0 || doc_freq == 0 {
        return 0.0;
    }
    let tf = tf as f64 / doc_len as f64;
    let idf = ((total_docs as f64 + 1.0) / (doc_freq as f64 + 1.0)).ln();
    tf * idf
}

// turns page contnet into a snippet containing queried terms
pub fn snippet(text: &str, query_terms: &[String]) -> String {
    let lower = text.to_lowercase();
    let byte_start = query_terms
        .iter()
        .filter_map(|term| lower.find(term))
        .min()
        .unwrap_or(0);

    let mut chars_seen = 0usize;
    let start = text
        .char_indices()
        .find_map(|(byte_index, _)| {
            if byte_index >= byte_start.saturating_sub(80) {
                Some(byte_index)
            } else {
                None
            }
        })
        .unwrap_or(0);

    let end = text[start..]
        .char_indices()
        .find_map(|(byte_index, _)| {
            chars_seen += 1;
            if chars_seen >= 240 {
                Some(start + byte_index)
            } else {
                None
            }
        })
        .unwrap_or(text.len());

    text[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// self explanatory
pub fn stop_words() -> HashSet<&'static str> {
    STOP_WORDS.iter().copied().collect()
}

static STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in", "is", "it",
    "its", "of", "on", "or", "that", "the", "to", "was", "were", "will", "with",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_normalizes_punctuation_case_and_stop_words() {
        let tokens = tokenize("The Rust-powered Search Engine!");
        assert_eq!(tokens, vec!["rustpowered", "search", "engine"]);
    }

    #[test]
    fn bm25_rewards_matching_terms() {
        let score = bm25(3, 100, 80.0, 10, 2);
        assert!(score > 0.0);
    }

    #[test]
    fn tfidf_ignores_missing_terms() {
        assert_eq!(tfidf(0, 100, 10, 2), 0.0);
    }

    #[test]
    fn snippet_returns_query_context() {
        let text = "alpha beta gamma rust search engine delta epsilon";
        let result = snippet(text, &["search".to_owned()]);
        assert!(result.contains("search engine"));
    }

    #[test]
    fn crawl_record_serializes_v2_outcomes() {
        let record = CrawlRecord {
            schema_version: 2,
            requested_url: Url::parse("https://example.com/").unwrap(),
            final_url: None,
            source_seed: Url::parse("https://example.com/").unwrap(),
            referrer: None,
            depth: 0,
            outcome: CrawlOutcome::RobotsBlocked,
            skip_reason: Some(CrawlSkipReason::RobotsTxt),
            title: None,
            status: None,
            content_type: None,
            content_length: None,
            content_hash: None,
            content_path: None,
            extracted_text: String::new(),
            links: Vec::new(),
            fetched_at_ms: 1,
        };

        let encoded = serde_json::to_string(&record).unwrap();
        assert!(encoded.contains("\"schema_version\":2"));
        assert!(encoded.contains("\"outcome\":\"robots_blocked\""));
        assert!(encoded.contains("\"skip_reason\":\"robots_txt\""));
    }
}
