/*
 * Some guidelines for the query engine
 * you will need to
 * Normalize the query
 * search for the phrases
 * use tfidf scoring
 * use pagerank algorithim in scoring
 * rank the pages based on these two metrics
 *
 */
use anyhow::{Result, bail};
use rusqlite::{Connection, ToSql};
use std::cmp::Ordering;
use std::collections::HashMap;

// structs
//
// TODO: add a to_string function as am impl for thsi struct
pub struct SearchResult {
    pub url: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub score: f64,
}

pub struct PostingMatch {
    pub page_id: i64,
    pub url: String,
    pub title: Option<String>,
    pub term_frequency: i64,
    pub word_count: i64,
    pub doc_frequency: i64,
    pub rank: f64,
}

// search the database for top k urls
pub fn search_db(pool: &Connection, query: &str, top_k: usize) -> Result<Vec<SearchResult>> {
    // normalize the query
    let normalized_query = remove_stop_words(query);

    if normalized_query.is_empty() {
        bail!("no valid terms in search query");
    }

    let total_pages: i64 = pool.query_row("SELECT COUNT(*) FROM pages", [], |row| row.get(0))?;

    if total_pages == 0 {
        return Ok(Vec::new());
    }
    // sql -> join tables so we can easily access needed information.
    let placeholders = vec!["?"; normalized_query.len()].join(", ");
    let sql = format!(
        r#"
        WITH doc_frequency AS (
            SELECT
                term_id,
                COUNT(*) AS document_frequency
            FROM postings
            GROUP BY term_id
        )
        SELECT
            pages.id AS page_id,
            pages.url AS url,
            pages.title AS title,
            page_contents.extracted_text AS extracted_text,
            postings.term_frequency AS term_frequency,
            document_stats.word_count AS word_count,
            doc_frequency.document_frequency AS doc_frequency,
            COALESCE(page_rank.rank, 1.0) AS rank
        FROM postings
        JOIN terms
            ON postings.term_id = terms.id
        JOIN pages
            ON postings.page_id = pages.id
        JOIN document_stats
            ON postings.page_id = document_stats.page_id
        LEFT JOIN page_contents
            ON postings.page_id = page_contents.page_id
        JOIN doc_frequency
            ON postings.term_id = doc_frequency.term_id
        LEFT JOIN page_rank
            ON postings.page_id = page_rank.page_id
        WHERE terms.term IN ({})
        "#,
        placeholders
    );
    // get the sql data that matches the term
    let query_params: Vec<&dyn ToSql> = normalized_query
        .iter()
        .map(|term| term as &dyn ToSql)
        .collect();

    let mut stmt = pool.prepare(&sql)?;

    let rows = stmt.query_map(query_params.as_slice(), |row| {
        Ok(PostingMatch {
            page_id: row.get("page_id")?,
            url: row.get("url")?,
            title: row.get("title")?,
            term_frequency: row.get("term_frequency")?,
            word_count: row.get("word_count")?,
            doc_frequency: row.get("doc_frequency")?,
            rank: row.get("rank")?,
        })
    })?;

    let mut results_by_page: HashMap<i64, SearchResult> = HashMap::new();

    // calculate tfidf
    for row in rows {
        let row = row?;

        if row.word_count <= 0 {
            continue;
        }

        let tf = row.term_frequency as f64 / row.word_count as f64;
        let idf = ((total_pages as f64 + 1.0) / (row.doc_frequency as f64 + 1.0)).ln();
        let tfidf = tf * idf;

        let entry = results_by_page
            .entry(row.page_id)
            .or_insert_with(|| SearchResult {
                url: row.url,
                title: row.title,
                snippet: None,
                score: 0.0,
            });

        entry.score += tfidf * row.rank;
    }

    // return a vector of top k search results
    let mut results: Vec<SearchResult> = results_by_page.into_values().collect();

    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

    results.truncate(top_k);

    Ok(results)
}

//
// within your query function youll handel page rank and tfidf
// TODO add a helper for making snippets
pub fn remove_stop_words(query: &str) -> Vec<String> {
    let stop_words = stop_words::get(stop_words::LANGUAGE::English);

    query
        .split_whitespace()
        .map(normalize_token)
        .filter(|term| !term.is_empty())
        .filter(|term| !stop_words.contains(&term.as_str()))
        .collect()
}

pub fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}
