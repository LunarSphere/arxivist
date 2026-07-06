//idexing bassically computes info about the page such as whats linking to it, how many terms it has, the freuqncy of tokens, save the rank of the page.
//
//
//
// the flow of indexing it a lot of creating variables to be saved to a struct or written to the file system. whihc is basically all code btu its fairly straight forward here.
// and the names expalin as well the comments.

use anyhow::{Context, Result};
use arxivist_core::{
    CrawlOutcome, CrawlRecord, DEFAULT_DOC_SHARD_SIZE, DEFAULT_POSTINGS_SHARD_COUNT, DocumentShard,
    IndexedDocument, MAX_POSTING_POSITIONS, Posting, PostingsShard, SHARDED_INDEX_SCHEMA_VERSION,
    SearchIndex, ShardedDocument, ShardedIndexManifest, ShardedTermStats, TermStats,
    document_shard_for_id, postings_shard_for_term, term_frequencies, tokenize,
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
};
use url::Url;

const MAX_INDEX_SNIPPET_TEXT_BYTES: usize = 16 * 1024;

pub struct IndexInput {
    pub final_url: Url,
    pub title: Option<String>,
    pub extracted_text: String,
}

pub fn build_index(records: Vec<CrawlRecord>, page_ranks: HashMap<Url, f64>) -> SearchIndex {
    // inputs are crawl records that were stored
    let inputs = records
        .into_iter()
        .filter(|record| record.outcome == CrawlOutcome::Stored)
        .filter_map(|record| {
            record.final_url.map(|final_url| IndexInput {
                final_url,
                title: record.title,
                extracted_text: record.extracted_text,
            })
        })
        .collect();
    // use page ranks and inputs to index the page.
    build_index_from_inputs(inputs, page_ranks)
}
// why is there so much abstraction for this
pub fn build_index_from_inputs(
    inputs: Vec<IndexInput>,
    page_ranks: HashMap<Url, f64>,
) -> SearchIndex {
    build_index_from_iter(inputs, page_ranks)
}

pub fn build_index_from_iter<I>(inputs: I, page_ranks: HashMap<Url, f64>) -> SearchIndex
where
    I: IntoIterator<Item = IndexInput>,
{
    let mut documents = Vec::new(); // list of pages taht will become searchable
    let mut document_frequency: HashMap<String, usize> = HashMap::new(); //how many documents contia na term at least once
    let mut inverted_index: HashMap<String, Vec<Posting>> = HashMap::new(); // map words to matching urls speeds up search drastically.
    let mut total_tokens = 0usize; // searchable body tokesn across all indexed documents.

    for input in inputs {
        // if nothing is there skip
        if input.extracted_text.trim().is_empty() {
            continue;
        }

        let tokens = tokenize(&input.extracted_text);
        let token_count = tokens.len();
        let term_freqs = term_frequencies(&tokens);
        let body_positions = term_positions(&tokens);
        let title_term_freqs = input
            .title
            .as_deref()
            .map(tokenize)
            .map(|tokens| term_frequencies(&tokens))
            .unwrap_or_default();
        let url_term_freqs = term_frequencies(&tokenize(&url_search_text(&input.final_url)));
        let mut snippet_text = input.extracted_text;
        truncate_string(&mut snippet_text, MAX_INDEX_SNIPPET_TEXT_BYTES);
        total_tokens += token_count;
        let document_id = documents.len();

        let mut document_terms = HashSet::new();
        document_terms.extend(term_freqs.keys().cloned());
        document_terms.extend(title_term_freqs.keys().cloned());
        document_terms.extend(url_term_freqs.keys().cloned());

        // Document frequency counts each term once per document, even when the
        // term appears in multiple searchable fields.
        for term in document_terms {
            *document_frequency.entry(term.clone()).or_insert(0) += 1;
            inverted_index
                .entry(term.clone())
                .or_default()
                .push(Posting {
                    document_id,
                    term_frequency: term_freqs.get(&term).copied().unwrap_or(0),
                    body_frequency: term_freqs.get(&term).copied().unwrap_or(0),
                    title_frequency: title_term_freqs.get(&term).copied().unwrap_or(0),
                    url_frequency: url_term_freqs.get(&term).copied().unwrap_or(0),
                    body_positions: body_positions.get(&term).cloned().unwrap_or_default(),
                });
        }
        // record our index for the docuement
        documents.push(IndexedDocument {
            id: document_id,
            url: input.final_url.clone(),
            title: input.title,
            text: snippet_text,
            token_count,
            term_freqs,
            title_term_freqs,
            url_term_freqs,
            page_rank: page_ranks.get(&input.final_url).copied().unwrap_or(1.0),
        });
    }

    let average_doc_len = if documents.is_empty() {
        0.0
    } else {
        total_tokens as f64 / documents.len() as f64
    };

    let terms = document_frequency
        .into_iter()
        .map(|(term, document_frequency)| (term, TermStats { document_frequency }))
        .collect();

    SearchIndex {
        documents,
        terms,
        inverted_index,
        average_doc_len,
    }
}

fn term_positions(tokens: &[String]) -> HashMap<String, Vec<u32>> {
    let mut positions = HashMap::new();
    for (position, token) in tokens.iter().enumerate() {
        let term_positions = positions.entry(token.clone()).or_insert_with(Vec::new);
        if term_positions.len() < MAX_POSTING_POSITIONS {
            term_positions.push(position as u32);
        }
    }
    positions
}

// small index useful if memory utilization is a concern.
pub fn write_sharded_index(
    search_index: &SearchIndex,
    output_dir: &Path,
    version: &str,
) -> Result<()> {
    // Sharded indexes are a directory of small JSON files instead of one large
    // index.json. The search API can then load only the shards needed for a query.
    fs::create_dir_all(output_dir.join("postings"))
        .with_context(|| format!("create postings shard dir {}", output_dir.display()))?;
    fs::create_dir_all(output_dir.join("docs"))
        .with_context(|| format!("create document shard dir {}", output_dir.display()))?;

    // Documents are grouped by numeric id so a posting can jump directly to the
    // shard containing the matching document metadata.
    let doc_shard_count = search_index
        .documents
        .len()
        .div_ceil(DEFAULT_DOC_SHARD_SIZE);
    let mut doc_shards: Vec<Vec<ShardedDocument>> = vec![Vec::new(); doc_shard_count.max(1)];
    for doc in &search_index.documents {
        let shard = document_shard_for_id(doc.id, DEFAULT_DOC_SHARD_SIZE);
        doc_shards[shard].push(ShardedDocument {
            id: doc.id,
            url: doc.url.clone(),
            title: doc.title.clone(),
            text: doc.text.clone(),
            token_count: doc.token_count,
            page_rank: doc.page_rank,
        });
    }

    for (shard, documents) in doc_shards.into_iter().enumerate() {
        write_json_file(
            &output_dir.join("docs").join(format!("{shard}.json")),
            &DocumentShard { documents },
        )?;
    }

    // Terms are grouped by hash. terms.json records which postings shard owns
    // each term, so query-time lookup avoids scanning all posting files.
    let mut sharded_terms = HashMap::new();
    let mut postings_shards: Vec<HashMap<String, Vec<Posting>>> =
        vec![HashMap::new(); DEFAULT_POSTINGS_SHARD_COUNT];
    for (term, stats) in &search_index.terms {
        let shard = postings_shard_for_term(term, DEFAULT_POSTINGS_SHARD_COUNT);
        sharded_terms.insert(
            term.clone(),
            ShardedTermStats {
                document_frequency: stats.document_frequency,
                postings_shard: shard,
            },
        );

        if let Some(postings) = search_index.inverted_index.get(term) {
            postings_shards[shard].insert(term.clone(), postings.clone());
        }
    }

    for (shard, postings) in postings_shards.into_iter().enumerate() {
        write_json_file(
            &output_dir.join("postings").join(format!("{shard}.json")),
            &PostingsShard { postings },
        )?;
    }

    // The manifest is the entrypoint for the search API. It describes the shard
    // counts and corpus stats needed to score results without loading everything.
    write_json_file(&output_dir.join("terms.json"), &sharded_terms)?;
    write_json_file(
        &output_dir.join("manifest.json"),
        &ShardedIndexManifest {
            schema_version: SHARDED_INDEX_SCHEMA_VERSION,
            version: version.to_owned(),
            document_count: search_index.documents.len(),
            term_count: search_index.terms.len(),
            average_doc_len: search_index.average_doc_len,
            doc_shard_size: DEFAULT_DOC_SHARD_SIZE,
            doc_shard_count: doc_shard_count.max(1),
            postings_shard_count: DEFAULT_POSTINGS_SHARD_COUNT,
            terms_path: "terms.json".to_owned(),
            generated_at_ms: now_ms(),
        },
    )?;

    Ok(())
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut writer = BufWriter::new(
        File::create(path).with_context(|| format!("create index artifact {}", path.display()))?,
    );
    serde_json::to_writer(&mut writer, value)
        .with_context(|| format!("write index artifact {}", path.display()))?;
    writer
        .flush()
        .with_context(|| format!("flush index artifact {}", path.display()))?;
    Ok(())
}

pub fn version_from_time() -> String {
    now_ms().to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn url_search_text(url: &Url) -> String {
    let mut value = String::new();
    if let Some(host) = url.host_str() {
        value.push_str(host);
        value.push(' ');
    }
    value.push_str(url.path());

    value
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect()
}

fn truncate_string(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }

    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
}

#[cfg(test)]
mod tests {
    use super::*;
    use arxivist_core::CrawlSkipReason;

    #[test]
    fn build_index_ignores_skipped_records() {
        let stored_url = Url::parse("https://example.com/stored").unwrap();
        let records = vec![
            record(
                stored_url.clone(),
                CrawlOutcome::Stored,
                None,
                "Rust search content with enough terms to index.",
            ),
            record(
                Url::parse("https://example.com/skipped").unwrap(),
                CrawlOutcome::Skipped,
                Some(CrawlSkipReason::LikelyJavascriptRequired),
                "this should not be indexed",
            ),
        ];

        let index = build_index(records, HashMap::new());

        assert_eq!(index.documents.len(), 1);
        assert_eq!(index.documents[0].url, stored_url);
        assert!(index.terms.contains_key("rust"));
        assert!(!index.terms.contains_key("should"));
        assert!(index.inverted_index.contains_key("rust"));
        assert!(!index.inverted_index.contains_key("should"));
    }

    #[test]
    fn build_index_limits_stored_snippet_text() {
        let url = Url::parse("https://example.com/large").unwrap();
        let records = vec![record(
            url,
            CrawlOutcome::Stored,
            None,
            &"searchable ".repeat(20_000),
        )];

        let index = build_index(records, HashMap::new());

        assert_eq!(index.documents.len(), 1);
        assert!(index.documents[0].text.len() <= MAX_INDEX_SNIPPET_TEXT_BYTES);
        assert!(
            index.documents[0].token_count > index.documents[0].text.split_whitespace().count()
        );
    }

    #[test]
    fn build_index_persists_postings_with_term_frequency() {
        let first_url = Url::parse("https://example.com/first").unwrap();
        let second_url = Url::parse("https://example.com/second").unwrap();
        let records = vec![
            record(first_url, CrawlOutcome::Stored, None, "rust rust search"),
            record(second_url, CrawlOutcome::Stored, None, "rust index"),
        ];

        let index = build_index(records, HashMap::new());
        let postings = index.inverted_index.get("rust").unwrap();

        assert_eq!(postings.len(), 2);
        assert_eq!(postings[0].document_id, 0);
        assert_eq!(postings[0].term_frequency, 2);
        assert_eq!(postings[0].body_frequency, 2);
        assert_eq!(postings[0].body_positions, vec![0, 1]);
        assert_eq!(postings[1].document_id, 1);
        assert_eq!(postings[1].term_frequency, 1);
        assert_eq!(postings[1].body_frequency, 1);
        assert_eq!(postings[1].body_positions, vec![0]);
    }

    #[test]
    fn build_index_caps_stored_body_positions() {
        let record = record(
            Url::parse("https://example.com/repeated").unwrap(),
            CrawlOutcome::Stored,
            None,
            &"rust ".repeat(MAX_POSTING_POSITIONS + 10),
        );

        let index = build_index(vec![record], HashMap::new());
        let posting = &index.inverted_index.get("rust").unwrap()[0];

        assert_eq!(posting.body_frequency, MAX_POSTING_POSITIONS + 10);
        assert_eq!(posting.body_positions.len(), MAX_POSTING_POSITIONS);
        assert_eq!(posting.body_positions[0], 0);
        assert_eq!(
            posting.body_positions[MAX_POSTING_POSITIONS - 1],
            (MAX_POSTING_POSITIONS - 1) as u32
        );
    }

    #[test]
    fn build_index_persists_title_only_postings() {
        let mut record = record(
            Url::parse("https://example.com/ordinary").unwrap(),
            CrawlOutcome::Stored,
            None,
            "Body text has enough unrelated words to pass the storage filter.",
        );
        record.title = Some("Quantum Retrieval Guide".to_owned());

        let index = build_index(vec![record], HashMap::new());
        let postings = index.inverted_index.get("quantum").unwrap();

        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].body_frequency, 0);
        assert_eq!(postings[0].title_frequency, 1);
        assert_eq!(postings[0].url_frequency, 0);
    }

    #[test]
    fn build_index_persists_url_only_postings() {
        let record = record(
            Url::parse("https://example.com/topics/neural-retrieval").unwrap(),
            CrawlOutcome::Stored,
            None,
            "Body text has enough unrelated words to pass the storage filter.",
        );

        let index = build_index(vec![record], HashMap::new());
        let postings = index.inverted_index.get("neural").unwrap();

        assert_eq!(postings.len(), 1);
        assert_eq!(postings[0].body_frequency, 0);
        assert_eq!(postings[0].title_frequency, 0);
        assert_eq!(postings[0].url_frequency, 1);
    }

    #[test]
    fn build_index_counts_document_frequency_once_per_document() {
        let mut record = record(
            Url::parse("https://example.com/rust/rust").unwrap(),
            CrawlOutcome::Stored,
            None,
            "rust rust search content with enough terms to index.",
        );
        record.title = Some("Rust Rust".to_owned());

        let index = build_index(vec![record], HashMap::new());

        assert_eq!(index.terms.get("rust").unwrap().document_frequency, 1);
    }

    #[test]
    fn sharded_index_round_trips_manifest_terms_postings_and_docs() {
        let url = Url::parse("https://example.com/rust").unwrap();
        let index = build_index(
            vec![record(
                url,
                CrawlOutcome::Stored,
                None,
                "rust rust search content with enough terms to index",
            )],
            HashMap::new(),
        );
        let output_dir =
            std::env::temp_dir().join(format!("arxivist-sharded-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&output_dir);

        write_sharded_index(&index, &output_dir, "test-version").unwrap();

        let manifest: arxivist_core::ShardedIndexManifest =
            serde_json::from_slice(&std::fs::read(output_dir.join("manifest.json")).unwrap())
                .unwrap();
        let terms: HashMap<String, arxivist_core::ShardedTermStats> =
            serde_json::from_slice(&std::fs::read(output_dir.join("terms.json")).unwrap()).unwrap();
        let rust_shard = terms.get("rust").unwrap().postings_shard;
        let postings: arxivist_core::PostingsShard = serde_json::from_slice(
            &std::fs::read(
                output_dir
                    .join("postings")
                    .join(format!("{rust_shard}.json")),
            )
            .unwrap(),
        )
        .unwrap();
        let docs: arxivist_core::DocumentShard =
            serde_json::from_slice(&std::fs::read(output_dir.join("docs/0.json")).unwrap())
                .unwrap();

        assert_eq!(manifest.version, "test-version");
        assert_eq!(manifest.document_count, 1);
        assert_eq!(manifest.schema_version, SHARDED_INDEX_SCHEMA_VERSION);
        assert_eq!(terms.get("rust").unwrap().document_frequency, 1);
        assert_eq!(postings.postings.get("rust").unwrap()[0].body_frequency, 2);
        assert_eq!(
            postings.postings.get("rust").unwrap()[0].body_positions,
            vec![0, 1]
        );
        assert_eq!(docs.documents[0].url.as_str(), "https://example.com/rust");

        let _ = std::fs::remove_dir_all(output_dir);
    }

    fn record(
        url: Url,
        outcome: CrawlOutcome,
        skip_reason: Option<CrawlSkipReason>,
        text: &str,
    ) -> CrawlRecord {
        CrawlRecord {
            schema_version: 2,
            requested_url: url.clone(),
            final_url: Some(url.clone()),
            source_seed: url,
            referrer: None,
            depth: 0,
            outcome,
            skip_reason,
            title: None,
            status: Some(200),
            content_type: Some("text/html".to_owned()),
            content_length: Some(10),
            content_hash: None,
            content_path: None,
            extracted_payload_path: None,
            extracted_text: text.to_owned(),
            links: Vec::new(),
            fetched_at_ms: 1,
        }
    }
}
