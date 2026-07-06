// record page content fills out appropriate structs
use crate::{
    args::Args,
    extract,
    filters::{self, MIN_TEXT_CHARS},
    types::{PageSnapshot, QueueItem},
};
use anyhow::{Context, Result};
use arxivist_core::{
    CrawlExtractedPayload, CrawlOutcome, CrawlRecord, CrawlSkipReason, content_hash,
};
use scraper::Html;
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::warn;
use url::Url;
// this function writes url page content to the file system
pub fn from_snapshot(args: &Args, item: &QueueItem, snapshot: PageSnapshot) -> CrawlRecord {
    let html = snapshot.html.clone();
    let mut record = from_snapshot_with_content_path(item, snapshot, None);
    if record.outcome == CrawlOutcome::Stored {
        let hash = record
            .content_hash
            .clone()
            .unwrap_or_else(|| content_hash(&html));
        let content_path = format!("content/{hash}.html"); // create a hash and that will be the path for page content
        if let Err(error) = fs::write(args.output_dir.join(&content_path), &html) {
            warn!(path = %content_path, ?error, "failed to write page content");
        } else {
            record.content_path = Some(content_path);
        }

        let extracted_path = format!("extracted/{hash}.json"); // where we stored extracted page contnet aka extracted text and links
        let payload = CrawlExtractedPayload {
            extracted_text: record.extracted_text.clone(),
            links: record.links.clone(),
        };
        // file i/o
        if let Some(parent) = args.output_dir.join(&extracted_path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Err(error) = fs::write(
            args.output_dir.join(&extracted_path),
            serde_json::to_vec(&payload).unwrap_or_default(),
        ) {
            warn!(path = %extracted_path, ?error, "failed to write extracted payload");
        } else {
            record.extracted_payload_path = Some(extracted_path);
        }
    }

    record
}

pub fn from_snapshot_with_content_path(
    item: &QueueItem,
    snapshot: PageSnapshot,
    content_path: Option<String>,
) -> CrawlRecord {
    let document = Html::parse_document(&snapshot.html);
    let title = extract::title(&document);
    let extracted_text = extract::text(&document);
    let links = extract::links(&snapshot.final_url, &document);
    let content_length = Some(snapshot.html.len() as u64);

    if !filters::is_html(&snapshot.content_type, &snapshot.html) {
        return skipped(
            item,
            snapshot,
            links,
            CrawlSkipReason::NonHtml,
            extracted_text,
        );
    }

    if filters::is_explicitly_non_english(&document) {
        return skipped(
            item,
            snapshot,
            links,
            CrawlSkipReason::NonEnglish,
            extracted_text,
        );
    }

    if filters::looks_javascript_required(&snapshot.html, &extracted_text) {
        return skipped(
            item,
            snapshot,
            links,
            CrawlSkipReason::LikelyJavascriptRequired,
            extracted_text,
        );
    }

    if extracted_text.trim().chars().count() < MIN_TEXT_CHARS {
        return skipped(
            item,
            snapshot,
            links,
            CrawlSkipReason::EmptyText,
            extracted_text,
        );
    }

    let hash = content_hash(&snapshot.html);

    CrawlRecord {
        schema_version: 2,
        requested_url: item.url.clone(),
        final_url: Some(snapshot.final_url),
        source_seed: item.source_seed.clone(),
        referrer: item.referrer.clone(),
        depth: item.depth,
        outcome: CrawlOutcome::Stored,
        skip_reason: None,
        title,
        status: Some(snapshot.status),
        content_type: snapshot.content_type,
        content_length,
        content_hash: Some(hash),
        content_path,
        extracted_payload_path: None,
        extracted_text,
        links,
        fetched_at_ms: now_ms(),
    }
}

pub fn skipped(
    item: &QueueItem,
    snapshot: PageSnapshot,
    links: Vec<Url>,
    reason: CrawlSkipReason,
    extracted_text: String,
) -> CrawlRecord {
    CrawlRecord {
        schema_version: 2,
        requested_url: item.url.clone(),
        final_url: Some(snapshot.final_url),
        source_seed: item.source_seed.clone(),
        referrer: item.referrer.clone(),
        depth: item.depth,
        outcome: CrawlOutcome::Skipped,
        skip_reason: Some(reason),
        title: None,
        status: Some(snapshot.status),
        content_type: snapshot.content_type,
        content_length: Some(snapshot.html.len() as u64),
        content_hash: None,
        content_path: None,
        extracted_payload_path: None,
        extracted_text,
        links,
        fetched_at_ms: now_ms(),
    }
}

pub fn diagnostic(
    item: &QueueItem,
    outcome: CrawlOutcome,
    reason: Option<CrawlSkipReason>,
) -> CrawlRecord {
    CrawlRecord {
        schema_version: 2,
        requested_url: item.url.clone(),
        final_url: None,
        source_seed: item.source_seed.clone(),
        referrer: item.referrer.clone(),
        depth: item.depth,
        outcome,
        skip_reason: reason,
        title: None,
        status: None,
        content_type: None,
        content_length: None,
        content_hash: None,
        content_path: None,
        extracted_payload_path: None,
        extracted_text: String::new(),
        links: Vec::new(),
        fetched_at_ms: now_ms(),
    }
}

pub fn append(output_dir: &Path, record: &CrawlRecord) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_dir.join("pages.jsonl"))
        .context("open crawl metadata file")?;

    serde_json::to_writer(&mut file, record)?;
    writeln!(file)?;
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
