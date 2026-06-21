use anyhow::Result;
use std::collections::HashMap;
use tracing::info;

mod handle_db;
use handle_db::*;
// we can index 5000 pages in 6 minutes
#[tokio::main]
async fn main() -> Result<()> {
    const DAMP: f64 = 0.85;

    tracing_subscriber::fmt::init();

    info!("starting indexer");

    let pool = connect_db().await?;
    info!("connected to database");

    // Bounded channel prevents main from creating a huge backlog while SQLite is still writing.
    let (tx, rx) = async_channel::bounded(10_000);

    let db_pool = pool.clone();
    let db_task = tokio::spawn(async move { db_writer(db_pool, rx).await });
    info!("started db writer task");

    let pages = load_pages_to_index(&pool).await?;
    info!(page_count = pages.len(), "loaded pages to index");

    for page in pages {
        info!(page_id = page.page_id, "indexing page");

        let words: Vec<&str> = page.extracted_text.split_whitespace().collect();
        let word_count = words.len() as i64;

        let mut unique_map: HashMap<String, i64> = HashMap::new();

        for word in words {
            unique_map
                .entry(word.to_string())
                .and_modify(|count| *count += 1)
                .or_insert(1);
        }

        let unique_term_count = unique_map.len() as i64;

        tx.send(DbEvent::DocumentStats {
            page_id: page.page_id,
            word_count,
            unique_terms: unique_term_count,
        })
        .await?;

        for (term, freq) in unique_map {
            // Do NOT info-log every term. That makes indexing much slower.
            tx.send(DbEvent::TermFrequency {
                page_id: page.page_id,
                term,
                term_frequency: freq,
            })
            .await?;
        }
    }

    info!("finished sending db events");

    drop(tx);
    info!("closed db event sender");

    db_task.await??;
    info!("db writer finished");

    page_rank(&pool, DAMP).await?;
    info!("page rank finished");

    pool.close().await;
    info!("database pool closed");

    Ok(())
}
