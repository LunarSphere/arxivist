use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use spider::website::Website;

use crate::{
    config::SpiderSettings,
    extract::extract_page,
    ids::{sha256_hex, url_hash},
    models::{CrawlStats, PageRecord, PageStatus},
    service::{CrawlExecution, CrawlRunner},
    storage::DynStorage,
};

pub struct SpiderCrawlRunner {
    store: DynStorage,
    settings: SpiderSettings,
}

impl SpiderCrawlRunner {
    pub fn new(store: DynStorage, settings: SpiderSettings) -> Self {
        Self { store, settings }
    }
}

//implements the page crawling loop. I'm already familiar with this.
#[async_trait]
impl CrawlRunner for SpiderCrawlRunner {
    async fn run(&self, execution: CrawlExecution) -> Result<CrawlStats> {
        let mut stats = CrawlStats::default();

        for seed_url in &execution.request.seed_urls {
            if stats.pages_fetched >= execution.request.max_pages as u64 {
                break;
            }

            let remaining = execution.request.max_pages - stats.pages_fetched as u32;
            let seed_stats = crawl_seed(
                self.store.clone(),
                &self.settings,
                &execution.crawl_id,
                seed_url,
                remaining,
                execution.request.depth_limit,
            )
            .await?;

            stats.pages_fetched += seed_stats.pages_fetched;
            stats.pages_failed += seed_stats.pages_failed;
        }

        Ok(stats)
    }
}
// crawl one of our seeded urls?
async fn crawl_seed(
    store: DynStorage,
    settings: &SpiderSettings,
    crawl_id: &str,
    seed_url: &str,
    max_pages: u32,
    depth_limit: usize,
) -> Result<CrawlStats> {
    let mut website = Website::new(seed_url);
    website
        .with_limit(max_pages)
        .with_depth(depth_limit)
        .with_respect_robots_txt(true)
        .with_return_page_links(true)
        .with_user_agent(Some(settings.user_agent.as_str()))
        .with_delay(settings.delay_ms)
        .with_request_timeout(Some(settings.request_timeout))
        .with_crawl_timeout(Some(settings.crawl_timeout));

    let mut rx = website.subscribe(128);
    let crawl_id = crawl_id.to_string();
    let subscriber = tokio::spawn(async move {
        let mut stats = CrawlStats::default();
        while let Ok(page) = rx.recv().await {
            match persist_page(store.clone(), &crawl_id, &page).await {
                Ok(PageStatus::Fetched) => stats.pages_fetched += 1,
                Ok(PageStatus::Failed | PageStatus::Skipped) => stats.pages_failed += 1,
                Err(err) => {
                    stats.pages_failed += 1;
                    tracing::error!(error = %err, "failed to persist crawled page");
                }
            }
        }
        stats
    });

    website.crawl().await;
    website.unsubscribe();

    Ok(subscriber.await?)
}

// crawl urls that we find from the seed?
async fn persist_page(
    store: DynStorage,
    crawl_id: &str,
    page: &spider::page::Page,
) -> Result<PageStatus> {
    let url = page.get_url().to_string();
    let url_hash = url_hash(&url);
    let http_status = Some(page.status_code.as_u16());
    let content_type = page
        .headers
        .as_ref()
        .and_then(|headers| headers.get("content-type"))
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_string());

    if !page.status_code.is_success() || page.error_status.is_some() {
        let status = PageStatus::Failed;
        store
            .put_page(PageRecord {
                crawl_id: crawl_id.to_string(),
                url_hash,
                url,
                status,
                http_status,
                content_type,
                title: None,
                s3_key: None,
                content_hash: None,
                links: Vec::new(),
                assets: Vec::new(),
                text_preview: None,
                word_count: 0,
                error: page
                    .error_status
                    .clone()
                    .or_else(|| Some(format!("HTTP {}", page.status_code))),
                fetched_at: Utc::now(),
            })
            .await?;
        return Ok(status);
    }

    let html = page.get_content();
    if html.trim().is_empty() {
        let status = PageStatus::Skipped;
        store
            .put_page(PageRecord {
                crawl_id: crawl_id.to_string(),
                url_hash,
                url,
                status,
                http_status,
                content_type,
                title: None,
                s3_key: None,
                content_hash: None,
                links: Vec::new(),
                assets: Vec::new(),
                text_preview: None,
                word_count: 0,
                error: Some("empty page body".to_string()),
                fetched_at: Utc::now(),
            })
            .await?;
        return Ok(status);
    }

    let extracted = extract_page(&url, &html);
    let content_hash = sha256_hex(html.as_bytes());
    let s3_key = store.put_raw_html(crawl_id, &url_hash, html).await?;
    let status = PageStatus::Fetched;

    store
        .put_page(PageRecord {
            crawl_id: crawl_id.to_string(),
            url_hash,
            url,
            status,
            http_status,
            content_type,
            title: extracted.title,
            s3_key: Some(s3_key),
            content_hash: Some(content_hash),
            links: extracted.links,
            assets: extracted.assets,
            text_preview: extracted.text_preview,
            word_count: extracted.word_count,
            error: None,
            fetched_at: Utc::now(),
        })
        .await?;

    Ok(status)
}
