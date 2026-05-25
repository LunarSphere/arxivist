use anyhow::Result;
use sqlx::{Executor, SqlitePool, sqlite::SqlitePoolOptions};
use std::collections::HashMap;
use tracing::info;

#[derive(sqlx::FromRow)]
pub struct PageToIndex {
    pub page_id: i64,
    pub extracted_text: String,
}

/*
This function will populate a page_rank table with
ranked pages
ALGORITHIM
1. start with a set of pages
2. crawl the web to determine link structure (links table)
3. assign each page an initial rank of 1/N w/ N = total_number of pages
4. UPDATE: for each current_page. SUM(attatched_page rank/ # links from attatched page)
*/
pub async fn page_rank(pool: &SqlitePool, damp: f64) -> Result<()> {
    // create a table to represent the page ranks
    sqlx::query(
        r#"
            CREATE TABLE IF NOT EXISTS page_rank (
                page_id INTEGER PRIMARY KEY,
                rank REAL NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY(page_id) REFERENCES pages(id)
            );
            "#,
    )
    .execute(pool)
    .await?;

    //fiil in vars for the page rank algorithim
    let total_pages: i64 = sqlx::query_scalar(
        r#"
            SELECT COUNT(*)
            FROM pages;
            "#,
    )
    .fetch_one(pool)
    .await?;

    if total_pages == 0 {
        return Ok(());
    }

    // get the page ids
    let page_ids: Vec<i64> = sqlx::query_scalar(
        r#"
            SELECT id
            FROM pages;
            "#,
    )
    .fetch_all(pool)
    .await?;

    // backlinks : to_page_id -> Vec<from_page_id>
    let rows: Vec<(i64, i64)> = sqlx::query_as(
        r#"
            SELECT to_page_id, from_page_id
            FROM links;
            "#,
    )
    .fetch_all(pool)
    .await?;

    let init_rank: f64 = 1.0 / (total_pages as f64);
    let mut back_links: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut out_link_counts: HashMap<i64, usize> = HashMap::new();

    for (to_page_id, from_page_id) in rows {
        back_links // tracks which pages link to this page
            .entry(to_page_id)
            .or_insert_with(Vec::new)
            .push(from_page_id);

        *out_link_counts.entry(from_page_id).or_insert(0) += 1; // tracks how many pages this page links to
    }

    let mut page_ranks: HashMap<i64, f64> = HashMap::new();

    for page_id in &page_ids {
        page_ranks.insert(*page_id, init_rank);
    }

    for _ in 0..20 {
        let mut next_page_ranks: HashMap<i64, f64> = HashMap::new();

        for page_id in &page_ids {
            let mut score = 0.0;

            if let Some(values) = back_links.get(page_id) {
                for val in values {
                    let rank = page_ranks.get(val).copied().unwrap_or(0.0);
                    let out_count = out_link_counts.get(val).copied().unwrap_or(1);

                    score += rank / out_count as f64;
                }
            }

            next_page_ranks.insert(*page_id, (1.0 - damp) / total_pages as f64 + (damp * score));
        }

        page_ranks = next_page_ranks;
    }

    let mut tx = pool.begin().await?;

    for (page_id, rank) in page_ranks {
        sqlx::query(
            r#"
            INSERT INTO page_rank (
                page_id,
                rank,
                updated_at
            )
            VALUES (?, ?, datetime('now'))
            ON CONFLICT(page_id) DO UPDATE SET
                rank = excluded.rank,
                updated_at = datetime('now');
            "#,
        )
        .bind(page_id)
        .bind(rank)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn load_pages_to_index(pool: &SqlitePool) -> Result<Vec<PageToIndex>> {
    let pages = sqlx::query_as::<_, PageToIndex>(
        r#"
        SELECT
            pages.id AS page_id,
            page_contents.extracted_text AS extracted_text
        FROM pages
        JOIN page_contents ON pages.id = page_contents.page_id
        WHERE page_contents.extracted_text IS NOT NULL
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(pages)
}

pub async fn connect_db() -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite://../data/crawler.db?mode=rwc")
        .await?;

    pool.execute("PRAGMA journal_mode = WAL;").await?;
    pool.execute("PRAGMA synchronous = NORMAL;").await?;
    pool.execute("PRAGMA temp_store = MEMORY;").await?;
    pool.execute("PRAGMA cache_size = -64000;").await?;

    Ok(pool)
}

pub enum DbEvent {
    DocumentStats {
        page_id: i64,
        word_count: i64,
        unique_terms: i64,
    },
    TermFrequency {
        page_id: i64,
        term: String,
        term_frequency: i64,
    },
}

pub async fn db_writer(
    pool: SqlitePool,
    rx: async_channel::Receiver<DbEvent>,
) -> anyhow::Result<()> {
    let mut term_cache: HashMap<String, i64> = HashMap::new();
    let mut processed = 0_i64;

    loop {
        let first_event = match rx.recv().await {
            Ok(event) => event,
            Err(_) => break,
        };

        let mut batch = vec![first_event];

        while batch.len() < 1000 {
            match rx.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }

        let mut tx = pool.begin().await?;

        for event in batch {
            processed += 1;

            match event {
                DbEvent::DocumentStats {
                    page_id,
                    word_count,
                    unique_terms,
                } => {
                    sqlx::query(
                        r#"
                        INSERT INTO document_stats (
                            page_id,
                            word_count,
                            unique_terms
                        )
                        VALUES (?, ?, ?)
                        ON CONFLICT(page_id) DO UPDATE SET
                            word_count = excluded.word_count,
                            unique_terms = excluded.unique_terms,
                            indexed_at = datetime('now');
                        "#,
                    )
                    .bind(page_id)
                    .bind(word_count)
                    .bind(unique_terms)
                    .execute(&mut *tx)
                    .await?;
                }

                DbEvent::TermFrequency {
                    page_id,
                    term,
                    term_frequency,
                } => {
                    let term_id = if let Some(id) = term_cache.get(&term) {
                        *id
                    } else {
                        sqlx::query(
                            r#"
                            INSERT OR IGNORE INTO terms (term)
                            VALUES (?);
                            "#,
                        )
                        .bind(&term)
                        .execute(&mut *tx)
                        .await?;

                        let term_id: i64 = sqlx::query_scalar(
                            r#"
                            SELECT id FROM terms
                            WHERE term = ?;
                            "#,
                        )
                        .bind(&term)
                        .fetch_one(&mut *tx)
                        .await?;

                        term_cache.insert(term, term_id);
                        term_id
                    };

                    sqlx::query(
                        r#"
                        INSERT INTO postings (
                            page_id,
                            term_id,
                            term_frequency
                        )
                        VALUES (?, ?, ?)
                        ON CONFLICT(page_id, term_id) DO UPDATE SET
                            term_frequency = excluded.term_frequency;
                        "#,
                    )
                    .bind(page_id)
                    .bind(term_id)
                    .bind(term_frequency)
                    .execute(&mut *tx)
                    .await?;
                }
            }
        }

        tx.commit().await?;

        if processed % 10_000 == 0 {
            info!(processed, "db writer processed events");
        }
    }

    info!(processed, "db writer finished processing events");

    Ok(())
}
