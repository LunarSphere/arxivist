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

use crate::arxiv_scrape::DbEvent;

//  CrawlerState holds everything that must be shared across
//  all concurrent crawl tasks.  It is wrapped in an `Arc` so
//  it can be cloned cheaply and sent to many tasks.
pub struct CrawlerState {
    pub client: Client,
    pub visited: DashSet<String>,
    pub limiter: governor::DefaultDirectRateLimiter,
    pub max_pages: usize,
}

impl CrawlerState {
    /// `requests_per_second` — how many HTTP requests we are
    /// allowed to make every second across ALL tasks combined.
    pub fn new(requests_per_second: u32, max_pages: usize) -> Self {
        let client = Client::builder()
            .user_agent("Archivist/toySearchengine-1.0")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        let max_pages = max_pages;
        Self {
            client,
            visited: DashSet::new(),
            limiter,
            max_pages,
        }
    }
}

struct InFlightGuard(Arc<AtomicUsize>);
impl InFlightGuard {
    fn new(counter: Arc<AtomicUsize>) -> Self {
        Self(counter)
    }
}
impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
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
    db_tx: Sender<DbEvent>,
    in_flight: Arc<AtomicUsize>,
) -> Result<()> {
    let _inflight_guard = InFlightGuard::new(Arc::clone(&in_flight));
    // Await the rate limiter.
    state.limiter.until_ready().await;

    // Send the GET request and await the response.
    let response = state.client.get(url.clone()).send().await?;

    //check for failed urls and non text/html
    if !response.status().is_success() {
        tracing::warn!(url = %url, status = %response.status(), "non-success status, skipping");
        //tell database the url failed
        let _ = db_tx
            .send(DbEvent::PageFailed {
                url: url.to_string(),
                error: format!("Http {}", response.status()),
            })
            .await;

        return Ok(());
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html") {
        return Ok(());
    }

    //TODO: we've gotten a repsonse so tell the db
    let _ = db_tx
        .send(DbEvent::PageFetched {
            url: url.to_string(),
            http_status: response.status().as_u16(),
            content_type: content_type.to_string(),
        })
        .await;

    // Await the body text.
    let body: String = response.text().await?;

    // Parse HTML and extract links.
    let (links, db_events) = {
        let document = Html::parse_document(&body);
        // Build the CSS selector for anchor tags
        let selector = Selector::parse("a[href]").unwrap();

        let mut links = Vec::new();
        let mut db_events = Vec::new();
        for element in document.select(&selector) {
            if state.visited.len() >= state.max_pages {
                break;
            }

            if let Some(href) = element.value().attr("href") {
                if let Ok(absolute) = url.join(href) {
                    if absolute.scheme() == "http" || absolute.scheme() == "https" {
                        let key = absolute.to_string();
                        if state.visited.insert(key.clone()) {
                            //TODO: tell the db the page was queued and the link was found
                            db_events.push(DbEvent::PageQueued { url: key.clone() });
                            db_events.push(DbEvent::LinkFound {
                                from_url: url.to_string(),
                                to_url: key.clone(),
                            });
                            if state.visited.len() < state.max_pages {
                                links.push(absolute);
                            }
                        }
                    }
                }
            }
        }
        (links, db_events)
    };
    //if messing with async and handling data you need an await or else data can be skipped.
    for event in db_events {
        let _ = db_tx.send(event).await;
    }
    // Send all links to the channel.
    for link in links {
        in_flight.fetch_add(1, Ordering::SeqCst);
        let _ = link_tx.send(link).await;
    }

    info!(url = %url, "crawled successfully");

    // Mark this task as done.
    Ok(())
}

//  Each worker loops forever, receiving URLs from the shared
//  channel and calling `crawl_page` on each one.
// The loop exits when the channel is closed (all senders have been dropped)

pub async fn worker(
    state: Arc<CrawlerState>,
    link_rx: Receiver<Url>,
    link_tx: Sender<Url>,
    db_tx: Sender<DbEvent>,
    in_flight: Arc<AtomicUsize>,
) {
    while let Ok(link) = link_rx.recv().await {
        let state = Arc::clone(&state);
        let tx = link_tx.clone();
        let db_tx = db_tx.clone();
        let counter = Arc::clone(&in_flight);

        let crawl_future = crawl_page(state, link, tx, db_tx, counter);
        tokio::spawn(async move {
            if let Err(e) = crawl_future.await {
                tracing::error!("crawl failed: {e}");
            }
        });
    }
}

// you cannot call an .await during an HTLM parsing loop. it is not thread safe.
