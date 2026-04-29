// ============================================================
//  Async Web Crawler Lab — Starter Code
//  CS [XXX]: Systems Programming with Rust
// ============================================================
//
//  LEARNING OBJECTIVES
//  -------------------
//  By the end of this lab you will be able to:
//    1. Explain the difference between synchronous and async I/O.
//    2. Write async functions using `async fn` and `.await`.
//    3. Spawn concurrent tasks with `tokio::spawn`.
//    4. Share state safely across tasks using `Arc`.
//    5. Use a rate limiter to be a "polite" crawler.
//    6. Extract links from HTML and build a crawl frontier.
//
//  INSTRUCTIONS
//  ------------
//  Search for every comment that starts with  👉 TODO  and
//  implement the missing code.  The program should compile and
//  run with zero TODOs remaining.
//
//  Run with:
//    cargo run
//
//  Expected final output (approximate):
//    INFO crawler: crawled url=https://example.com
//    INFO crawler: crawled url=...
//    Finished crawling 42 unique URLs
// ============================================================

use anyhow::Result;
use async_channel::{Receiver, Sender};
use dashmap::DashSet;
use governor::{Quota, RateLimiter};
use reqwest::Client;
use scraper::{Html, Selector};
use std::num::NonZeroU32;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tracing::info;
use url::Url;

//  `CrawlerState` holds everything that must be shared across
//  all concurrent crawl tasks.  It is wrapped in an `Arc` so
//  it can be cloned cheaply and sent to many tasks.
struct CrawlerState {
    client: Client,
    visited: DashSet<String>,
    limiter: governor::DefaultDirectRateLimiter,
}

impl CrawlerState {
    /// `requests_per_second` — how many HTTP requests we are
    /// allowed to make every second across ALL tasks combined.
    fn new(requests_per_second: u32) -> Self {
        let client = Client::builder()
            .user_agent("CrawlerLab/1.0 (university course)")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota =
            Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        Self {
            client,
            visited: DashSet::new(),
            limiter,
        }
    }
}

// ============================================================
//  PART 2 — FETCHING A PAGE  (async fn)
//  ============================================================
//
//  This is the core async function.  It:
//    1. Waits for a rate-limit token.
//    2. Sends an HTTP GET request and awaits the response.
//    3. Checks the status code and Content-Type.
//    4. Parses the HTML and extracts every <a href="…"> link.
//    5. Sends newly-discovered absolute URLs down `link_tx`.

async fn crawl_page(
    state: Arc<CrawlerState>,
    url: Url,
    link_tx: Sender<Url>,
    in_flight: Arc<AtomicUsize>,
) -> Result<()> {
    // Await the rate limiter.
    state.limiter.until_ready().await;

    // Send the GET request and await the response.
    let response = state.client.get(url.clone()).send().await?;

    //check for failed urls and non text/html
    if !response.status().is_success() {
        tracing::warn!(url = %url, status = %response.status(), "non-success status, skipping");
        in_flight.fetch_sub(1, Ordering::SeqCst);
        return Ok(());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html"){
        in_flight.fetch_sub(1, Ordering::SeqCst);
        return Ok(())
    }

    // Await the body text.
    let body: String = response.text().await?;

    // Parse HTML and extract links.
    let links = {
        let document = Html::parse_document(&body);
        // Build the CSS selector for anchor tags
        let selector = Selector::parse("a[href]").unwrap();

        let mut links = Vec::new();
        for element in document.select(&selector) {
            if let Some(href) = element.value().attr("href"){
                if let Ok(absolute) = url.join(href){
                    if absolute.scheme() == "http" || absolute.scheme() == "https"{
                        let key = absolute.to_string();
                        if state.visited.insert(key){
                            links.push(absolute);
                        }
                    }
                }
            }
        }
        links
    }; // document dropped here before any await

    for link in links {
        let _ = link_tx.send(link).await;
    }

    info!(url = %url, "crawled successfully");

    // Mark this task as done.
    in_flight.fetch_sub(1, Ordering::SeqCst);
    Ok(())
}


//  Each worker loops forever, receiving URLs from the shared
//  channel and calling `crawl_page` on each one.
// The loop exits when the channel is closed (all senders have been dropped)

async fn worker(
    state: Arc<CrawlerState>,
    link_rx: Receiver<Url>,
    link_tx: Sender<Url>,
    in_flight: Arc<AtomicUsize>,){
    while let Ok(link) = link_rx.recv().await {
        let state = Arc::clone(&state);
        let tx = link_tx.clone();
        let counter = Arc::clone(&in_flight);

        let crawl_future = crawl_page(state, link, tx, counter);
        tokio::spawn(async move {
            if let Err(e) = crawl_future.await {
                tracing::error!("crawl failed: {e}");
            }
        });
    }
}


// ============================================================
//  PART 4 — MAIN: wire everything together
//  ============================================================

#[tokio::main]
async fn main() -> Result<()> {
    // logging
    tracing_subscriber::fmt::init();

    // ----------------------------------------------------------
    // Configuration — feel free to adjust these values.
    // ----------------------------------------------------------
    let requests_per_second: u32 = 10;  // stay polite
    let num_workers: usize      = 20;   // concurrent tasks
    let seed_urls = vec![
        "https://example.com",
        // Add more seed URLs here if you like.
    ];

    // ----------------------------------------------------------
    // STEP 4-A: Create shared state and channel.
    //
    // `async_channel::bounded` creates a channel with a fixed
    // buffer.  Multiple producers (workers discovering links)
    // and multiple consumers (workers fetching pages) can use
    // the same channel — unlike `tokio::mpsc`.
    // ----------------------------------------------------------

    let state = Arc::new(CrawlerState::new(requests_per_second));

    //Create a bounded async-channel with a capacity of 10_000 URLs.
    //let (link_tx, link_rx) = async_channel::bounded(10_000);
    let (link_tx, link_rx): (Sender<Url>, Receiver<Url>) = async_channel::bounded(10000);

    // `in_flight` counts URLs that have been queued but not yet
    // fully processed.  When it reaches 0, crawling is done.
    let in_flight = Arc::new(AtomicUsize::new(0));


    //Seed the crawl queue.
    for seed in seed_urls{
        let url = Url::parse(&seed)?;
        state.visited.insert(url.to_string());
        link_tx.send(url).await?;
        in_flight.fetch_add(1, Ordering::SeqCst);
    }

    // ----------------------------------------------------------
    // STEP 4-C: Spawn worker tasks.
    //
    // Use `tokio::spawn` to launch `num_workers` async tasks.
    // Each task gets a clone of:
    //   • `state`      (Arc — cheap clone)
    //   • `link_rx`    (Receiver — clone allowed by async-channel)
    //   • `link_tx`    (Sender — clone allowed by async-channel)
    //   • `in_flight`  (Arc — cheap clone)
    //
    // Collect the JoinHandles so we can await them later.
    // ----------------------------------------------------------

    // 👉 TODO (4-C): Spawn `num_workers` worker tasks.
    let mut handles = Vec::new();
    for _ in 0..num_workers {
        let state = Arc::clone(&state);
        let link_rx = link_rx.clone();
        let link_tx = link_tx.clone();
        let in_flight = Arc::clone(&in_flight);
        handles.push(tokio::spawn(worker(state, link_rx, link_tx, in_flight)));
    }

    // ----------------------------------------------------------
    // STEP 4-D: Wait for crawling to finish.
    //
    // Drop the original `link_tx` here.  The workers hold their
    // own clones; once they are all done AND `in_flight` hits 0,
    // every sender will be dropped and `link_rx.recv()` will
    // return `Err`, causing workers to exit.
    //
    // Then await all JoinHandles.
    // ----------------------------------------------------------

    // 👉 TODO (4-D): Drop link_tx and await all handles.
    drop(link_tx);

    for handle in handles {
        handle.await.unwrap();
    }

    println!(
        "\nFinished crawling {} unique URLs",
        state.visited.len()
    );

    Ok(())
}

// ============================================================
//  BONUS CHALLENGES  (implement after the core lab is working)
// ============================================================