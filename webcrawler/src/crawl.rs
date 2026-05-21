use anyhow::Result; // failures will be returned alongside the result
use async_channel::{Receiver, Sender}; // allows concurrent communication between tasks
use dashmap::DashSet; // thread-safe set for tracking visited URLs **replaces hashset**
use governor::{Quota, RateLimiter}; // rate limiting for HTTP requests
use reqwest::Client; // HTTP client for making requests
use scraper::{Html, Selector}; // HTML parsing and CSS selector matching
use sha2::{Digest, Sha256};
use std::num::NonZeroU32;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tracing::info;
use url::Url; // URL parsing

use crate::arxiv_scrape::DbEvent;

pub struct CrawlItem {
    url: Url,
    pub crawl_depth: usize,
}
impl CrawlItem {
    pub fn new(url: Url, crawl_depth: usize) -> Self {
        let url = url;
        let crawl_depth = crawl_depth;
        Self { url, crawl_depth }
    }
}

//  CrawlerState holds everything that must be shared across
//  all concurrent crawl tasks.  It is wrapped in an `Arc` so
//  it can be cloned cheaply and sent to many tasks.
pub struct CrawlerState {
    pub client: Client,
    pub visited: DashSet<String>,
    pub limiter: governor::DefaultDirectRateLimiter,
    pub max_pages: usize,
    pub depth_limit: usize,
}

impl CrawlerState {
    /// `requests_per_second` — how many HTTP requests we are
    /// allowed to make every second across ALL tasks combined.
    pub fn new(requests_per_second: u32, max_pages: usize, depth_limit: usize) -> Self {
        let client = Client::builder()
            .user_agent("Archivist/toySearchengine-1.0")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        let max_pages = max_pages;
        let depth_limit = depth_limit;
        Self {
            client,
            visited: DashSet::new(),
            limiter,
            max_pages,
            depth_limit,
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
    item: CrawlItem,
    link_tx: Sender<CrawlItem>,
    db_tx: Sender<DbEvent>,
    in_flight: Arc<AtomicUsize>,
) -> Result<()> {
    let _inflight_guard = InFlightGuard::new(Arc::clone(&in_flight));
    // Await the rate limiter.
    state.limiter.until_ready().await;

    // Send the GET request and await the response.
    let response = state.client.get(item.url.clone()).send().await?;

    //check for failed urls and non text/html
    if !response.status().is_success() {
        tracing::warn!(item.url = %item.url, status = %response.status(), "non-success status, skipping");
        //tell database the url failed
        let _ = db_tx
            .send(DbEvent::PageFailed {
                url: item.url.to_string(),
                error: format!("Http {}", response.status()),
            })
            .await;

        return Ok(());
    }

    let http_status = response.status().as_u16();
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !content_type.contains("text/html") {
        return Ok(());
    }

    // Await the body text.
    let body = response.text().await?;

    // Parse title from HTML.
    let title: Option<String> = {
        let document = Html::parse_document(&body);
        let title_selector = Selector::parse("title").unwrap();

        document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .filter(|title| !title.is_empty())
    };
    let extracted_text: Option<String> = {
        let document = Html::parse_document(&body);

        let text = document
            .root_element()
            .text()
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        Some(text).filter(|text| !text.is_empty())
    };
    let content_hash: String = hex::encode(Sha256::digest(body.as_bytes()));
    let _ = db_tx
        .send(DbEvent::PageFetched {
            url: item.url.clone().to_string(),
            http_status,
            content_type,
            title,
        })
        .await;
    let _ = db_tx
        .send(DbEvent::PageContentFound {
            url: item.url.to_string(),
            html: body.clone(),
            extracted_text: extracted_text,
            content_hash: Some(content_hash),
        })
        .await;
    // Parse HTML and extract links.
    let (links, db_events) = {
        let document = Html::parse_document(&body);
        // Build the CSS selector for anchor tags
        let selector = Selector::parse("a[href]").unwrap(); //originial link selector

        let mut links = Vec::new();
        let mut db_events = Vec::new();

        //populate links channel and send events to our sql database
        for element in document.select(&selector) {
            if state.visited.len() >= state.max_pages {
                break;
            }

            if let Some(href) = element.value().attr("href") {
                if let Ok(absolute) = item.url.join(href) {
                    if absolute.scheme() == "http" || absolute.scheme() == "https" {
                        if item.crawl_depth + 1 > state.depth_limit {
                            continue;
                        }
                        let key = absolute.to_string();
                        if state.visited.insert(key.clone()) {
                            let anchor_text = element.text().collect::<String>().trim().to_string();
                            //TODO: tell the db the page was queued and the link was found
                            db_events.push(DbEvent::PageQueued { url: key.clone() });
                            db_events.push(DbEvent::LinkFound {
                                from_url: item.url.to_string(),
                                to_url: key.clone(),
                                anchor_text: Some(anchor_text).filter(|text| !text.is_empty()),
                            });
                            let new_item = CrawlItem::new(absolute, item.crawl_depth + 1);
                            if state.visited.len() < state.max_pages {
                                links.push(new_item);
                            }
                        }
                    }
                }
            }
        }
        //We're gonna start finding assets starting with the Images
        let img_selector = Selector::parse("img[src]").unwrap();
        for element in document.select(&img_selector) {
            if let Some(src) = element.value().attr("src") {
                if let Ok(asset_url) = item.url.join(src) {
                    let alt_text = element.value().attr("alt").map(|s| s.trim().to_string());
                    db_events.push(DbEvent::AssetFound {
                        url: item.url.to_string(),
                        asset_url: asset_url.to_string(),
                        asset_type: "image".to_string(),
                        alt_text,
                    });
                }
            }
        }
        //next we do scripts
        let script_selector = Selector::parse("script[src]").unwrap();
        for element in document.select(&script_selector) {
            if let Some(src) = element.value().attr("src") {
                if let Ok(asset_url) = item.url.join(src) {
                    db_events.push(DbEvent::AssetFound {
                        url: item.url.to_string(),
                        asset_url: asset_url.to_string(),
                        asset_type: "script".to_string(),
                        alt_text: None,
                    });
                }
            }
        }
        //finally css sheets
        let css_selector = Selector::parse(r#"link[rel="stylesheet"]"#).unwrap();
        for element in document.select(&css_selector) {
            if let Some(src) = element.value().attr("src") {
                if let Ok(asset_url) = item.url.join(src) {
                    db_events.push(DbEvent::AssetFound {
                        url: item.url.to_string(),
                        asset_url: asset_url.to_string(),
                        asset_type: "stylesheet".to_string(),
                        alt_text: None,
                    });
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
    for item in links {
        in_flight.fetch_add(1, Ordering::SeqCst);
        let _ = link_tx.send(item).await;
    }

    info!(item.url = %item.url, "crawled successfully");

    // Mark this task as done.
    Ok(())
}

//  Each worker loops forever, receiving URLs from the shared
//  channel and calling `crawl_page` on each one.
// The loop exits when the channel is closed (all senders have been dropped)

pub async fn worker(
    state: Arc<CrawlerState>,
    link_rx: Receiver<CrawlItem>,
    link_tx: Sender<CrawlItem>,
    db_tx: Sender<DbEvent>,
    in_flight: Arc<AtomicUsize>,
) {
    while let Ok(item) = link_rx.recv().await {
        let state = Arc::clone(&state);
        let tx = link_tx.clone();
        let db_tx = db_tx.clone();
        let counter = Arc::clone(&in_flight);

        let crawl_future = crawl_page(state, item, tx, db_tx, counter);
        tokio::spawn(async move {
            if let Err(e) = crawl_future.await {
                tracing::error!("crawl failed: {e}");
            }
        });
    }
}

// you cannot call an .await during an HTLM parsing loop. it is not thread safe.
