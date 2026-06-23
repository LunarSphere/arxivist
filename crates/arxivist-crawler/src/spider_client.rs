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

pub async fn crawl_one(args: &Args, item: &QueueItem) -> CrawlRecord {
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
        .with_user_agent(Some(USER_AGENT))
        .with_on_link_blocked_callback(Some(move |_url| {
            blocked_flag.store(true, Ordering::Relaxed);
        }));

    let mut rx = website.subscribe(16);
    let page_collector = tokio::spawn(async move {
        let mut pages = Vec::new();
        while let Ok(result) = tokio::time::timeout(Duration::from_millis(250), rx.recv()).await {
            match result {
                Ok(page) => pages.push(page),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        pages
    });

    website.scrape().await;
    website.unsubscribe();
    let pages = page_collector.await.unwrap_or_default();

    if robots_blocked.load(Ordering::Relaxed) {
        return record::diagnostic(
            item,
            CrawlOutcome::RobotsBlocked,
            Some(CrawlSkipReason::RobotsTxt),
        );
    }

    let Some(snapshot) = pages
        .first()
        .and_then(|page| first_page_snapshot(page, &item.url))
    else {
        return record::diagnostic(
            item,
            CrawlOutcome::FetchFailed,
            Some(CrawlSkipReason::FetchError),
        );
    };

    record::from_snapshot(args, item, snapshot)
}

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
