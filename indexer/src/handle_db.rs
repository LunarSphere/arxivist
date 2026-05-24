use anyhow::Result;
use sqlx::{Executor, SqlitePool, sqlite::SqlitePoolOptions};
use std::collections::HashMap;
use tracing::info;

#[derive(sqlx::FromRow)]
pub struct PageToIndex {
    pub page_id: i64,
    pub extracted_text: String,
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
