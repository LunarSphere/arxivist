// our interactions with the spider library
use crate::{
    args::Args,
    record,
    types::{PageSnapshot, QueueItem},
};
use arxivist_core::{CrawlOutcome, CrawlRecord, CrawlSkipReason};
use spider::{page::Page, website::Website};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use url::Url;

const USER_AGENT: &str = "ArxivistCrawler/0.1 (+https://example.com/arxivist)";
// crawl a page and record it
pub async fn crawl_one(args: &Args, item: &QueueItem) -> CrawlRecord {
    match crawl_snapshot(args, item).await {
        Ok(snapshot) => record::from_snapshot(args, item, snapshot),
        Err(record) => record,
    }
}
// return page content and the record
pub async fn crawl_snapshot(args: &Args, item: &QueueItem) -> Result<PageSnapshot, CrawlRecord> {
    let robots_blocked = Arc::new(AtomicBool::new(false));
    let blocked_flag = Arc::clone(&robots_blocked);
    let mut website = Website::new(item.url.as_str());
    website
        .with_limit(1)
        .with_depth(1)
        .with_delay(args.delay_ms)
        .with_concurrency_limit(Some(args.concurrency.max(1)))
        .with_request_timeout(Some(Duration::from_secs(15)))
        .with_respect_robots_txt(true)
        .with_return_page_links(true)
        .with_user_agent(Some(USER_AGENT))
        .with_on_link_blocked_callback(Some(move |_url| {
            blocked_flag.store(true, Ordering::Relaxed);
        }));

    // Website::scrape collects pages internally; reading get_pages avoids racing the broadcast stream.
    website.scrape().await;

    if robots_blocked.load(Ordering::Relaxed) {
        return Err(record::diagnostic(
            item,
            CrawlOutcome::RobotsBlocked,
            Some(CrawlSkipReason::RobotsTxt),
        ));
    }

    if let Some(snapshot) = website
        .get_pages()
        .and_then(|pages| pages.first())
        .and_then(|page| first_page_snapshot(page, &item.url))
    {
        return Ok(snapshot);
    }

    // Keep local crawls useful when spider completes without publishing a Page for a reachable URL.
    if let Some(snapshot) = reqwest_snapshot(item).await {
        return Ok(snapshot);
    }

    Err(record::diagnostic(
        item,
        CrawlOutcome::FetchFailed,
        Some(CrawlSkipReason::FetchError),
    ))
}

// so spider has a complicated page. we convert the big data to only whats useful for us AKA pagesnapshot
fn first_page_snapshot(page: &Page, fallback_url: &Url) -> Option<PageSnapshot> {
    let html = page.get_html().to_string();
    let final_url = Url::parse(page.get_url()).unwrap_or_else(|_| fallback_url.clone());
    let content_type = page
        .headers
        .as_ref()
        .and_then(|headers| headers.get(reqwest::header::CONTENT_TYPE))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    Some(PageSnapshot {
        final_url,
        status: page.status_code.as_u16(),
        content_type,
        html,
    })
}

async fn reqwest_snapshot(item: &QueueItem) -> Option<PageSnapshot> {
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(15))
        .build()
        .ok()?;
    let response = client.get(item.url.clone()).send().await.ok()?;
    let final_url = response.url().clone();
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let html = response.text().await.ok()?;

    Some(PageSnapshot {
        final_url,
        status,
        content_type,
        html,
    })
}
