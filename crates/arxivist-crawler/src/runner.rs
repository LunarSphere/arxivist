use crate::{args::Args, filters, record, spider_client, types::QueueItem};
use anyhow::Result;
use arxivist_core::{CrawlOutcome, CrawlSkipReason};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    time::Duration,
};
use tracing::{info, warn};

pub async fn run(args: Args) -> Result<()> {
    fs::create_dir_all(args.output_dir.join("content"))?;

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new();
    for seed in &args.seeds {
        if visited.insert(seed.as_str().to_owned()) {
            queue.push_back(QueueItem {
                url: seed.clone(),
                source_seed: seed.clone(),
                referrer: None,
                depth: 0,
            });
        }
    }

    let mut written = 0usize;
    let mut stored = 0usize;
    let mut bad_hosts: HashMap<String, usize> = HashMap::new();
    let mut suppressed_hosts = HashSet::new();

    while let Some(item) = queue.pop_front() {
        if written >= args.max_pages {
            break;
        }

        let Some(host) = item.url.host_str().map(str::to_owned) else {
            continue;
        };

        if suppressed_hosts.contains(&host) {
            let record = record::diagnostic(
                &item,
                CrawlOutcome::HostSuppressed,
                Some(CrawlSkipReason::BadHostThreshold),
            );
            record::append(&args.output_dir, &record)?;
            written += 1;
            continue;
        }

        let record = spider_client::crawl_one(&args, &item).await;
        if filters::should_penalize(record.skip_reason) {
            let count = bad_hosts.entry(host.clone()).or_default();
            *count += 1;
            if *count >= args.bad_host_threshold {
                suppressed_hosts.insert(host.clone());
                warn!(
                    host,
                    count = *count,
                    "suppressing bad host for this crawl run"
                );
            }
        }

        if record.outcome == CrawlOutcome::Stored {
            stored += 1;
            if item.depth < args.max_depth {
                for link in &record.links {
                    let key = link.as_str().to_owned();
                    if visited.insert(key) && written + queue.len() < args.max_pages {
                        queue.push_back(QueueItem {
                            url: link.clone(),
                            source_seed: item.source_seed.clone(),
                            referrer: record.final_url.clone(),
                            depth: item.depth + 1,
                        });
                    }
                }
            }
        }

        info!(
            requested_url = %record.requested_url,
            outcome = ?record.outcome,
            stored,
            written = written + 1,
            "processed crawl record"
        );
        record::append(&args.output_dir, &record)?;
        written += 1;

        if args.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
        }
    }

    info!(stored, written, "crawl complete");
    Ok(())
}
