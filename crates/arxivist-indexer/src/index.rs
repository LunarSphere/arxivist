use arxivist_core::{
    CrawlOutcome, CrawlRecord, IndexedDocument, SearchIndex, TermStats, term_frequencies, tokenize,
};
use std::collections::HashMap;
use url::Url;

pub fn build_index(records: Vec<CrawlRecord>, page_ranks: HashMap<Url, f64>) -> SearchIndex {
    let mut documents = Vec::new();
    let mut document_frequency: HashMap<String, usize> = HashMap::new();
    let mut total_tokens = 0usize;

    for record in records {
        if record.outcome != CrawlOutcome::Stored || record.extracted_text.trim().is_empty() {
            continue;
        }

        let Some(final_url) = record.final_url.clone() else {
            continue;
        };

        let tokens = tokenize(&record.extracted_text);
        let token_count = tokens.len();
        let term_freqs = term_frequencies(&tokens);
        total_tokens += token_count;

        // Document frequency counts each term once per document.
        for term in term_freqs.keys() {
            *document_frequency.entry(term.clone()).or_insert(0) += 1;
        }

        documents.push(IndexedDocument {
            id: documents.len(),
            url: final_url.clone(),
            title: record.title,
            text: record.extracted_text,
            token_count,
            term_freqs,
            page_rank: page_ranks.get(&final_url).copied().unwrap_or(1.0),
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
        average_doc_len,
    }
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
            extracted_text: text.to_owned(),
            links: Vec::new(),
            fetched_at_ms: 1,
        }
    }
}
