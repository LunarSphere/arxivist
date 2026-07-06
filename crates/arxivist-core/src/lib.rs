// define structs, enums, and functions reused in other crates
// an Enum represents a value that is one of servral possible variants
// TLDR structs: CrawlRecord, search document, indexed document, term stats, ranked result
// TLDR enums: Crawl Outocome, Crawl Skip Reason
// TLDR fn: content_hash, tokeninze, normaize_token, term_frequencies, bm25, tfidf, snippet
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use url::Url;

pub const SHARDED_INDEX_SCHEMA_VERSION: u8 = 2;
pub const DEFAULT_DOC_SHARD_SIZE: usize = 1_000;
pub const DEFAULT_POSTINGS_SHARD_COUNT: usize = 256;
pub const MAX_POSTING_POSITIONS: usize = 256;

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
    #[serde(default)]
    pub extracted_payload_path: Option<String>,
    pub extracted_text: String,
    pub links: Vec<Url>,
    pub fetched_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlExtractedPayload {
    pub extracted_text: String,
    pub links: Vec<Url>,
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
    NonEnglish,
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
    #[serde(default)]
    pub inverted_index: HashMap<String, Vec<Posting>>,
    pub average_doc_len: f64,
}

impl SearchIndex {
    pub fn ensure_inverted_index(&mut self) {
        if self.inverted_index.is_empty() {
            self.rebuild_inverted_index();
        }
    }

    pub fn rebuild_inverted_index(&mut self) {
        self.inverted_index.clear();

        for doc in &self.documents {
            let mut terms = HashSet::new();
            terms.extend(doc.term_freqs.keys().cloned());
            terms.extend(doc.title_term_freqs.keys().cloned());
            terms.extend(doc.url_term_freqs.keys().cloned());

            for term in terms {
                let body_frequency = doc.term_freqs.get(&term).copied().unwrap_or(0);
                let title_frequency = doc.title_term_freqs.get(&term).copied().unwrap_or(0);
                let url_frequency = doc.url_term_freqs.get(&term).copied().unwrap_or(0);

                if body_frequency + title_frequency + url_frequency == 0 {
                    continue;
                }

                let body_positions = body_positions(&doc.text, &term);
                self.inverted_index.entry(term).or_default().push(Posting {
                    document_id: doc.id,
                    term_frequency: body_frequency,
                    body_frequency,
                    title_frequency,
                    url_frequency,
                    body_positions,
                });
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedDocument {
    pub id: usize,
    pub url: Url,
    pub title: Option<String>,
    pub text: String,
    pub token_count: usize,
    pub term_freqs: HashMap<String, usize>,
    #[serde(default)]
    pub title_term_freqs: HashMap<String, usize>,
    #[serde(default)]
    pub url_term_freqs: HashMap<String, usize>,
    pub page_rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermStats {
    pub document_frequency: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardedIndexManifest {
    pub schema_version: u8,
    pub version: String,
    pub document_count: usize,
    pub term_count: usize,
    pub average_doc_len: f64,
    pub doc_shard_size: usize,
    pub doc_shard_count: usize,
    pub postings_shard_count: usize,
    pub terms_path: String,
    pub generated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardedTermStats {
    pub document_frequency: usize,
    pub postings_shard: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardedDocument {
    pub id: usize,
    pub url: Url,
    pub title: Option<String>,
    pub text: String,
    pub token_count: usize,
    pub page_rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentShard {
    pub documents: Vec<ShardedDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostingsShard {
    pub postings: HashMap<String, Vec<Posting>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Posting {
    pub document_id: usize,
    #[serde(default)]
    pub term_frequency: usize,
    #[serde(default)]
    pub body_frequency: usize,
    #[serde(default)]
    pub title_frequency: usize,
    #[serde(default)]
    pub url_frequency: usize,
    #[serde(default)]
    pub body_positions: Vec<u32>,
}

impl Posting {
    pub fn body_frequency(&self) -> usize {
        if self.body_frequency == 0 && self.title_frequency == 0 && self.url_frequency == 0 {
            self.term_frequency
        } else {
            self.body_frequency
        }
    }

    pub fn total_frequency(&self) -> usize {
        self.body_frequency() + self.title_frequency + self.url_frequency
    }
}

pub fn body_positions(text: &str, term: &str) -> Vec<u32> {
    tokenize(text)
        .into_iter()
        .enumerate()
        .filter_map(|(position, token)| {
            if token == term {
                Some(position as u32)
            } else {
                None
            }
        })
        .take(MAX_POSTING_POSITIONS)
        .collect()
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
    idf * ((tf * (k1 + 1.0)) / (tf + k1 * length_norm)) // bm25 metric
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

pub fn postings_shard_for_term(term: &str, shard_count: usize) -> usize {
    if shard_count == 0 {
        return 0;
    }

    let mut hash = 2_166_136_261u32;
    for byte in term.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash as usize % shard_count
}

pub fn document_shard_for_id(document_id: usize, shard_size: usize) -> usize {
    document_id / shard_size.max(1)
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
    use std::collections::HashMap;

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
    fn search_index_rebuilds_inverted_index_from_documents() {
        let mut index = SearchIndex {
            documents: vec![IndexedDocument {
                id: 0,
                url: Url::parse("https://example.com/rust").unwrap(),
                title: None,
                text: "rust rust search".to_owned(),
                token_count: 3,
                term_freqs: HashMap::from([("rust".to_owned(), 2), ("search".to_owned(), 1)]),
                title_term_freqs: HashMap::from([("rust".to_owned(), 1)]),
                url_term_freqs: HashMap::new(),
                page_rank: 1.0,
            }],
            terms: HashMap::from([(
                "rust".to_owned(),
                TermStats {
                    document_frequency: 1,
                },
            )]),
            inverted_index: HashMap::new(),
            average_doc_len: 3.0,
        };

        index.ensure_inverted_index();
        let postings = index.inverted_index.get("rust").unwrap();

        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].document_id, 0);
        assert_eq!(postings[0].term_frequency, 2);
        assert_eq!(postings[0].body_frequency, 2);
        assert_eq!(postings[0].title_frequency, 1);
        assert_eq!(postings[0].body_positions, vec![0, 1]);
    }

    #[test]
    fn old_posting_json_defaults_body_positions() {
        let posting_json = r#"{
            "document_id": 0,
            "term_frequency": 2,
            "body_frequency": 2,
            "title_frequency": 0,
            "url_frequency": 0
        }"#;

        let posting: Posting = serde_json::from_str(posting_json).unwrap();

        assert!(posting.body_positions.is_empty());
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
            extracted_payload_path: None,
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
