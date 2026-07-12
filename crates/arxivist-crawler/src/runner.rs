// run crawler locally
use crate::{args::Args, filters, record, spider_client, types::QueueItem};
use anyhow::{Context, Result};
use arxivist_core::{CrawlOutcome, CrawlSkipReason};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    sync::Arc,
};
use tokio::{
    sync::{Mutex, Notify},
    task::JoinSet,
    time::{Duration, Instant},
};
use tracing::{info, warn};

pub async fn run(args: Args) -> Result<()> {
    if args.seeds.is_empty() {
        anyhow::bail!("at least one --seed is required when --storage local is used");
    }
    prepare_local_output(&args)?;

    let args = Arc::new(args);
    let state = Arc::new(CrawlState::new(&args));
    let mut workers = JoinSet::new();
    // spawn x workers with cloned arguments and state
    for worker_id in 0..args.concurrency.max(1) {
        let args = Arc::clone(&args);
        let state = Arc::clone(&state);
        workers.spawn(async move { worker(worker_id, args, state).await });
    }
    // if we stop getting a result from joining th workers then joinning them fialled
    while let Some(result) = workers.join_next().await {
        // join_next retuns the output of a worker once its task is complete
        result.context("local crawler worker task failed")??;
    }
    // once we are done print crawl complete to console
    let (stored, written) = state.summary().await;
    info!(stored, written, "crawl complete"); // how many pages we stored and how many pages we wrote
    Ok(())
}

// We want to do the crawl state this way so workers picking tasks from the state dont conflict with each other

// Shared local crawl state used by every async worker.
// The crawler has one frontier, one dedupe set, and one set of counters.
struct CrawlState {
    // Protects all mutable scheduling state so workers reserve and finish URLs atomically.
    inner: Mutex<CrawlStateInner>,
    // Wakes idle workers when another worker adds links or finishes the last in-flight item.
    notify: Notify,
    // Serializes writes to pages.jsonl; content files are content-addressed and can be written independently.
    output_lock: Mutex<()>,
}

// where the fields we wanna mutate are workers access them by locking
struct CrawlStateInner {
    // FIFO crawl frontier. Workers pop from here until it is empty or budgets are exhausted.
    queue: VecDeque<QueueItem>,
    // URL strings already seeded or queued. This keeps one crawl run from revisiting the same URL.
    visited: HashSet<String>,
    // Per-host failure counts for pages that are probably bad sources for this crawl.
    bad_hosts: HashMap<String, usize>,
    // Hosts that crossed the bad-host threshold and should no longer be fetched in this run.
    suppressed_hosts: HashSet<String>,
    // Per-host politeness schedule. Each host gets its own next allowed fetch time.
    host_next_allowed: HashMap<String, Instant>,
    // URLs handed to workers. This is the hard attempted-page budget, including in-flight work.
    reserved: usize,
    // Workers currently processing a URL. Used to know whether an empty queue means "done".
    in_flight: usize,
    // Records appended to pages.jsonl, including skipped and failed pages.
    written: usize,
    // Pages that passed filters and were saved as indexable content.
    stored: usize,
}

// Snapshot returned after a worker finishes a URL so logs do not need to hold the state lock.
struct FinishStats {
    stored: usize,
    written: usize,
    queue_depth: usize,
}

impl CrawlState {
    fn new(args: &Args) -> Self {
        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        // for each seed if its awllowed and not in visited push it into the queue
        for seed in &args.seeds {
            if filters::is_allowed_crawl_url(seed) && visited.insert(seed.as_str().to_owned()) {
                queue.push_back(QueueItem {
                    url: seed.clone(),
                    source_seed: seed.clone(),
                    referrer: None,
                    depth: 0,
                });
            }
        }

        Self {
            inner: Mutex::new(CrawlStateInner {
                queue,
                visited,
                bad_hosts: HashMap::new(),
                suppressed_hosts: HashSet::new(),
                host_next_allowed: HashMap::new(),
                reserved: 0,
                in_flight: 0,
                written: 0,
                stored: 0,
            }),
            notify: Notify::new(),
            output_lock: Mutex::new(()),
        }
    }
    // grabs next page from the innercrawlstate
    async fn next_item(&self, args: &Args) -> Option<QueueItem> {
        loop {
            let notified = self.notify.notified();
            {
                // if we we have more pages than our target ie 64k end the loop
                let mut inner = self.inner.lock().await;
                if inner.reserved >= args.max_pages
                    || args
                        .target_stored_pages
                        .is_some_and(|target| inner.stored >= target)
                {
                    return None;
                }
                // uptick metrics if we can still pop items from the queue return the item
                if let Some(item) = inner.queue.pop_front() {
                    inner.reserved += 1;
                    inner.in_flight += 1;
                    return Some(item);
                }
                // if no workers are working quit
                if inner.in_flight == 0 {
                    return None;
                }
            }
            //ELSE wait for notified  to finish
            notified.await;
        }
    }

    // check if a host is suppressed
    async fn is_suppressed(&self, host: &str) -> bool {
        let inner = self.inner.lock().await;
        inner.suppressed_hosts.contains(host)
    }

    // this function prevents multiple workers from from requesting the same domain
    async fn wait_for_host_slot(&self, host: &str, delay: Duration) {
        // if delay is zero fetch immediately
        if delay.is_zero() {
            return;
        }

        let slot = {
            let mut inner = self.inner.lock().await; // lock the shared crawl state
            let now = Instant::now();
            let slot = inner // figure out when hos is next allowed to be fetched
                .host_next_allowed
                .get(host)
                .copied()
                .unwrap_or(now)
                .max(now);
            inner
                .host_next_allowed
                .insert(host.to_owned(), slot + delay); // reserve a slot for this worker
            slot
        };

        tokio::time::sleep_until(slot).await; // slep until workers reserved slot
    }

    // when we are done with and item update innercrawlstates internal stats
    async fn finish_item(
        &self,
        args: &Args,
        item: &QueueItem,
        record: &arxivist_core::CrawlRecord,
    ) -> FinishStats {
        // a url without a host cannot affect per-host supression, but it still
        // counts as a completed record so workers can drain correctly
        let Some(host) = item.url.host_str().map(str::to_owned) else {
            return self.mark_written_only().await; //
        };

        let mut inner = self.inner.lock().await;
        // track bad pages, if a host gives enough bad pages then future queued urls from the host are recorded without
        // another fetch
        if filters::should_penalize(record.skip_reason) {
            let count = {
                let count = inner.bad_hosts.entry(host.clone()).or_default();
                *count += 1;
                *count
            };
            if count >= args.bad_host_threshold {
                inner.suppressed_hosts.insert(host.clone());
                warn!(host, count, "suppressing bad host for this crawl run");
            }
        }

        if record.outcome == CrawlOutcome::Stored {
            inner.stored += 1;
            if item.depth < args.max_depth {
                for link in &record.links {
                    let key = link.as_str().to_owned();
                    if filters::is_allowed_crawl_url(link)
                        && inner.visited.insert(key)
                        && inner.reserved + inner.queue.len() < args.max_pages
                    {
                        inner.queue.push_back(QueueItem {
                            url: link.clone(),
                            source_seed: item.source_seed.clone(),
                            referrer: record.final_url.clone(),
                            depth: item.depth + 1,
                        });
                    }
                }
            }
        }
        inner.written += 1;
        inner.in_flight = inner.in_flight.saturating_sub(1);
        let stats = FinishStats {
            stored: inner.stored,
            written: inner.written,
            queue_depth: inner.queue.len(),
        };
        drop(inner);
        self.notify.notify_waiters();
        stats
    }
    // fallback for when a page has no host
    async fn mark_written_only(&self) -> FinishStats {
        let mut inner = self.inner.lock().await;
        inner.written += 1;
        inner.in_flight = inner.in_flight.saturating_sub(1);
        let stats = FinishStats {
            stored: inner.stored,
            written: inner.written,
            queue_depth: inner.queue.len(),
        };
        drop(inner);
        self.notify.notify_waiters();
        stats
    }
    // reuturn how many pages we stored, and how many we have writen
    async fn summary(&self) -> (usize, usize) {
        let inner = self.inner.lock().await;
        (inner.stored, inner.written)
    }
}

async fn worker(worker_id: usize, args: Arc<Args>, state: Arc<CrawlState>) -> Result<()> {
    // how long to wait between requests to a host
    let host_delay = Duration::from_millis(args.delay_ms);
    // while we can get the next url from the queue
    while let Some(item) = state.next_item(&args).await {
        let host = item.url.host_str().map(str::to_owned);
        let record = match host.as_deref() {
            Some(host) if state.is_suppressed(host).await => record::diagnostic(
                // if host is supressed record
                &item,
                CrawlOutcome::HostSuppressed,
                Some(CrawlSkipReason::BadHostThreshold),
            ),
            Some(host) => {
                state.wait_for_host_slot(host, host_delay).await; // wait until we can request the host again
                spider_client::crawl_one(&args, &item).await // use spider to crawl the page
            }
            None => record::diagnostic(
                // if we failed to fetch a url from the queue record it
                &item,
                CrawlOutcome::FetchFailed,
                Some(CrawlSkipReason::FetchError),
            ),
        };
        // save results ro .jsonl
        let append_result = {
            let _guard = state.output_lock.lock().await;
            record::append(&args.output_dir, &record)
        };
        let stats = state.finish_item(&args, &item, &record).await;
        append_result?;
        // output crawl info to the terminal
        info!(
            worker_id,
            host = ?host,
            queue_depth = stats.queue_depth,
            requested_url = %record.requested_url,
            outcome = ?record.outcome,
            stored = stats.stored,
            written = stats.written,
            "processed crawl record"
        );
    }

    Ok(())
}

//create blank files to write data to
fn prepare_local_output(args: &Args) -> Result<()> {
    // Local adapters write metadata plus payload files so downstream tools can
    // run even after a crawl that finds no storable pages.
    fs::create_dir_all(args.output_dir.join("content"))?;
    fs::create_dir_all(args.output_dir.join("extracted"))?;
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(args.output_dir.join("pages.jsonl"))
        .context("create crawl metadata file")?; // jsonl file creation
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[tokio::test]
    async fn next_item_does_not_reserve_past_max_pages() {
        let mut args = args(vec![
            Url::parse("https://example.com/one").unwrap(),
            Url::parse("https://example.com/two").unwrap(),
        ]);
        args.max_pages = 1;
        let state = CrawlState::new(&args);

        assert!(state.next_item(&args).await.is_some());
        assert!(state.next_item(&args).await.is_none());
    }

    #[tokio::test]
    async fn finish_item_enqueues_links_until_depth_and_budget() {
        let args = args(vec![Url::parse("https://example.com/root").unwrap()]);
        let state = CrawlState::new(&args);
        let item = state.next_item(&args).await.unwrap();
        let record = stored_record(
            &item,
            vec![
                Url::parse("https://example.com/a").unwrap(),
                Url::parse("https://example.com/b").unwrap(),
            ],
        );

        let stats = state.finish_item(&args, &item, &record).await;

        assert_eq!(stats.written, 1);
        assert_eq!(stats.stored, 1);
        assert_eq!(stats.queue_depth, 2);
    }

    #[tokio::test]
    async fn finish_item_enqueues_only_allowed_english_links() {
        let args = args(vec![Url::parse("https://example.com/root").unwrap()]);
        let state = CrawlState::new(&args);
        let item = state.next_item(&args).await.unwrap();
        let record = stored_record(
            &item,
            vec![
                Url::parse("https://en.wikipedia.org/wiki/Rust").unwrap(),
                Url::parse("https://fr.wikipedia.org/wiki/Rouille").unwrap(),
            ],
        );

        let stats = state.finish_item(&args, &item, &record).await;

        assert_eq!(stats.queue_depth, 1);
    }

    #[tokio::test]
    async fn next_item_stops_at_target_stored_pages() {
        let mut args = args(vec![
            Url::parse("https://example.com/root").unwrap(),
            Url::parse("https://example.com/next").unwrap(),
        ]);
        args.target_stored_pages = Some(1);
        let state = CrawlState::new(&args);
        let item = state.next_item(&args).await.unwrap();
        let record = stored_record(&item, Vec::new());

        state.finish_item(&args, &item, &record).await;

        assert!(state.next_item(&args).await.is_none());
    }

    #[tokio::test]
    async fn host_delay_serializes_same_host_slots() {
        let args = args(vec![Url::parse("https://example.com/root").unwrap()]);
        let state = CrawlState::new(&args);
        let delay = Duration::from_millis(25);
        let first = Instant::now();
        state.wait_for_host_slot("example.com", delay).await;
        state.wait_for_host_slot("example.com", delay).await;

        assert!(first.elapsed() >= delay);
    }

    #[test]
    fn prepare_local_output_creates_empty_pages_jsonl() {
        let mut args = args(vec![Url::parse("https://example.com/root").unwrap()]);
        args.output_dir = temp_dir("arxivist-crawler-output");

        prepare_local_output(&args).unwrap();

        assert!(args.output_dir.join("content").is_dir());
        assert!(args.output_dir.join("extracted").is_dir());
        assert!(args.output_dir.join("pages.jsonl").is_file());

        let _ = std::fs::remove_dir_all(args.output_dir);
    }

    // nothing here is compiled in the main crawler. just dummy data
    fn args(seeds: Vec<Url>) -> Args {
        Args {
            seeds,
            max_pages: 10,
            target_stored_pages: None,
            crawl_id: "test".to_owned(),
            max_depth: 1,
            output_dir: std::env::temp_dir(),
            delay_ms: 0,
            concurrency: 2,
            bad_host_threshold: 3,
        }
    }

    fn stored_record(item: &QueueItem, links: Vec<Url>) -> arxivist_core::CrawlRecord {
        arxivist_core::CrawlRecord {
            schema_version: 2,
            requested_url: item.url.clone(),
            final_url: Some(item.url.clone()),
            source_seed: item.source_seed.clone(),
            referrer: item.referrer.clone(),
            depth: item.depth,
            outcome: CrawlOutcome::Stored,
            skip_reason: None,
            title: None,
            status: Some(200),
            content_type: Some("text/html".to_owned()),
            content_length: Some(100),
            content_hash: Some("hash".to_owned()),
            content_path: None,
            extracted_payload_path: None,
            extracted_text: "enough text for a stored crawl record".to_owned(),
            links,
            fetched_at_ms: 1,
        }
    }

    fn temp_dir(prefix: &str) -> std::path::PathBuf {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        std::env::temp_dir().join(format!("{prefix}-{now_ms}-{}", std::process::id()))
    }
}
