// /* 
// * I want to build a web crawler in rust, 
// * dependenceies I'll need tokio, reqwest, scraper, url 
// * web crawler process
// * start with seed url, fetch page, parse html, extract links, and content, add new urls to queue, repeat
// * respect robots.txt, we are crawling and scraping, crawliing is IO-bound so we need tokio for concurrency 
// * 
//  */
// // use thiserror::Error;
use select::document::Document;
use select::predicate::Name;
use url::Url;
use anyhow::Result;
use dashmap::DashSet; //we use this b/c its safe for multiple threads // replaces hashset. 
use governor::{Quota, RateLimiter};
use reqwest::Client;
use scraper::{Html, Selector};
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::mpsc; // a channel is a fancy queue designed for thread safe comms. it replaces deque 
use rand::Rng;
use tracing_subscriber;

//TODO: add error handling, 
// respect robots.txt ->
// add delay between requests, 
// limit crawl depth, 
// handle dynamic content, 
// add user-agent header, 
// implement concurrency with tokio tasks. 

//some pages are dynamic. the load their content with JS after page loads. 
// if we fetch a site like this with request we'll get bare htlm. because reqwest only makes html request
// to load the bage we need the API, a headless browserlike (selinium), scraping service. 

//bassically an object that represents a single crawler
struct CrawlerState{
    //track client visited limiter
    client:Client,
    visited:DashSet<String>,
    limiter: RateLimiter<
        governor::state::NotKeyed,
        governor::state::InMemoryState,
        governor::clock::DefaultClock,
    >
}

impl CrawlerState{
    // instantiates a crawler with an assigned requests per second, 
    // struct contains client, quota, and visted fields
    fn new(requests_per_second: u32) -> Self{
        let client = Client::builder()
            .user_agent("")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        Self{
            client,
            visited: DashSet::new(),
            limiter,
        }
    }
}




async fn crawl(
    state: Arc<CrawlerState>,
    url: Url,
    link_tx: mpsc::Sender<Url>,
) -> Result<()> {
    // Wait for rate limit token before making the request
    state.limiter.until_ready().await;

    let response = state.client.get(url.clone()).send().await?;

    // Only process successful HTML responses
    if !response.status().is_success() {
        tracing::warn!(url = %url, status = %response.status(), "non-success status");
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

    let body = response.text().await?;
    let document = Html::parse_document(&body);
    let selector = Selector::parse("a[href]").unwrap();

    for element in document.select(&selector) {
        if let Some(href) = element.value().attr("href") {
            // Resolve relative URLs against the current page
            if let Ok(absolute) = url.join(href) {
                // Only follow http/https links
                if absolute.scheme() == "http" || absolute.scheme() == "https" {
                    // Skip if already visited
                    let key = absolute.to_string();
                    if state.visited.insert(key) {
                        let _ = link_tx.send(absolute).await;
                    }
                }
            }
        }
    }

    tracing::info!(url = %url, "crawled successfully");
    Ok(())
}


// TODO: Implement the crawler yourself 
// TODO: setup scrape. itll acces a a channel view the site and save the page information to a sqlite database. 



//url lets us add relative urls to base urls
use std::sync::atomic::{AtomicUsize, Ordering};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let state = Arc::new(CrawlerState::new(50));
    let (link_tx, link_rx) = async_channel::bounded::<Url>(10_000);
    let in_flight = Arc::new(AtomicUsize::new(0));

    // Seed URLs
    for seed in ["https://example.com", "https://rust-lang.org"] {
        let url = Url::parse(seed)?;
        state.visited.insert(url.to_string());
        link_tx.send(url).await?;
        in_flight.fetch_add(1, Ordering::SeqCst);
    }

    // Spawn workers that compete for URLs from the shared channel
    let mut handles = Vec::new();
    for _ in 0..100 {
        let state = Arc::clone(&state);
        let rx = link_rx.clone();
        let tx = link_tx.clone();
        let counter = Arc::clone(&in_flight);

        handles.push(tokio::spawn(async move {
            while let Ok(url) = rx.recv().await {
                let _ = crawl(&state, url, &tx, &counter).await;
            }
        }));
    }

    // Close sender side - workers will exit when channel drains
    drop(link_tx);

    for handle in handles {
        handle.await?;
    }

    println!("Crawled {} unique URLs", state.visited.len());
    Ok(())
}



