use anyhow::Result; // failures will be returned alongside the result
use async_channel::{Receiver, Sender}; // allows concurrent communication between tasks
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use url::Url; // URL parsing

mod arxiv_scrape;
mod crawl;
use arxiv_scrape::*;
use crawl::*;

#[tokio::main]
async fn main() -> Result<()> {
    // logging
    tracing_subscriber::fmt::init();

    //configuration
    let requests_per_second: u32 = 10; // stay polite
    let num_workers: usize = 20; // concurrent tasks
    let max_pages: usize = 1000;
    let seed_urls = vec!["https://example.com"];

    let state = Arc::new(CrawlerState::new(requests_per_second, max_pages));

    //Create a bounded async-channel with a capacity of 10_000 URLs.
    //let (link_tx, link_rx) = async_channel::bounded(10_000);
    let (link_tx, link_rx): (Sender<Url>, Receiver<Url>) = async_channel::bounded(10000);
    let (db_tx, db_rx) = async_channel::bounded::<DbEvent>(10000);
    // `in_flight` counts URLs that have been queued but not yet
    // fully processed.  When it reaches 0, crawling is done.
    let in_flight = Arc::new(AtomicUsize::new(0));

    // create database
    let pool = setup_db().await?;

    //start the db writer
    let db_handle = tokio::spawn(async move {
        if let Err(e) = db_writer(pool, db_rx).await {
            tracing::error!("database writer failed: {e}");
        }
    });

    //Seed the crawl queue.
    for seed in seed_urls {
        let url = Url::parse(&seed)?;
        state.visited.insert(url.to_string());
        db_tx
            .send(DbEvent::PageQueued {
                url: url.to_string(),
            })
            .await?;
        link_tx.send(url).await?;
        in_flight.fetch_add(1, Ordering::SeqCst);
    }

    // Spawn `num_workers` worker tasks.
    let mut handles = Vec::new();
    for _ in 0..num_workers {
        let state = Arc::clone(&state);
        let link_rx = link_rx.clone();
        let db_tx = db_tx.clone();
        let link_tx = link_tx.clone();
        let in_flight = Arc::clone(&in_flight);
        handles.push(tokio::spawn(worker(
            state, link_rx, link_tx, db_tx, in_flight,
        )));
    }

    //Drop link_tx and await all handles.
    loop {
        if in_flight.load(Ordering::SeqCst) == 0 {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    for handle in &handles {
        handle.abort();
    }

    drop(link_tx);
    drop(db_tx);

    for handle in handles {
        let _ = handle.await;
    }

    let _ = db_handle.await;
    println!("\nFinished crawling {} unique URLs", state.visited.len());
    Ok(())
}
