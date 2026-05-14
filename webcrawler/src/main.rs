use anyhow::Result; // failures will be returned alongside the result
use async_channel::{Receiver, Sender}; // allows concurrent communication between tasks
use dashmap::DashSet; // thread-safe set for tracking visited URLs **replaces hashset**
use governor::{Quota, RateLimiter}; // rate limiting for HTTP requests
use reqwest::Client; // HTTP client for making requests
use scraper::{Html, Selector}; // HTML parsing and CSS selector matching
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tracing::info;
use url::Url; // URL parsing

//  CrawlerState holds everything that must be shared across
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
            .user_agent("Archivist/toySearchengine-1.0")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        Self {
            client,
            visited: DashSet::new(),
            limiter,
        }
    }
}
// Async function
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

    if !content_type.contains("text/html") {
        in_flight.fetch_sub(1, Ordering::SeqCst);
        return Ok(());
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
            if let Some(href) = element.value().attr("href") {
                if let Ok(absolute) = url.join(href) {
                    if absolute.scheme() == "http" || absolute.scheme() == "https" {
                        let key = absolute.to_string();
                        if state.visited.insert(key) {
                            links.push(absolute);
                        }
                    }
                }
            }
        }
        links
    };

    // Send all links to the channel.
    for link in links {
        in_flight.fetch_add(1, Ordering::SeqCst);
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
    in_flight: Arc<AtomicUsize>,
) {
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

#[tokio::main]
async fn main() -> Result<()> {
    // logging
    tracing_subscriber::fmt::init();

    //configuration
    let requests_per_second: u32 = 10; // stay polite
    let num_workers: usize = 20; // concurrent tasks
    let seed_urls = vec!["https://example.com"];

    let state = Arc::new(CrawlerState::new(requests_per_second));

    //Create a bounded async-channel with a capacity of 10_000 URLs.
    //let (link_tx, link_rx) = async_channel::bounded(10_000);
    let (link_tx, link_rx): (Sender<Url>, Receiver<Url>) = async_channel::bounded(10000);

    // `in_flight` counts URLs that have been queued but not yet
    // fully processed.  When it reaches 0, crawling is done.
    let in_flight = Arc::new(AtomicUsize::new(0));

    //Seed the crawl queue.
    for seed in seed_urls {
        let url = Url::parse(&seed)?;
        state.visited.insert(url.to_string());
        link_tx.send(url).await?;
        in_flight.fetch_add(1, Ordering::SeqCst);
    }

    // Spawn `num_workers` worker tasks.
    let mut handles = Vec::new();
    for _ in 0..num_workers {
        let state = Arc::clone(&state);
        let link_rx = link_rx.clone();
        let link_tx = link_tx.clone();
        let in_flight = Arc::clone(&in_flight);
        handles.push(tokio::spawn(worker(state, link_rx, link_tx, in_flight)));
    }

    //Drop link_tx and await all handles.
    drop(link_tx);
    for handle in handles {
        handle.await.unwrap();
    }
    println!("\nFinished crawling {} unique URLs", state.visited.len());
    Ok(())
}
