// i dont wanna write down how page ranks works again.
// it bassically prioritizes pages that have many bages linking to it.
use arxivist_core::{CrawlOutcome, CrawlRecord};
use std::collections::{HashMap, HashSet};

use url::Url;

pub fn compute_page_rank(
    records: &[CrawlRecord],
    damping: f64,
    iterations: usize,
) -> HashMap<Url, f64> {
    let pages: HashSet<Url> = records
        .iter()
        .filter(|record| record.outcome == CrawlOutcome::Stored)
        .filter_map(|record| record.final_url.clone())
        .collect();
    let page_count = pages.len();
    if page_count == 0 {
        return HashMap::new();
    }
    let mut outgoing: HashMap<Url, Vec<Url>> = HashMap::new();
    let mut incoming: HashMap<Url, Vec<Url>> = HashMap::new();
    for record in records {
        if record.outcome != CrawlOutcome::Stored {
            continue;
        }
        let Some(final_url) = record.final_url.clone() else {
            continue;
        };
        let links: Vec<Url> = record
            .links
            .iter()
            .filter(|link| pages.contains(*link))
            .cloned()
            .collect();
        for link in &links {
            incoming
                .entry(link.clone())
                .or_default()
                .push(final_url.clone());
        }
        outgoing.insert(final_url, links);
    }

    let initial = 1.0 / page_count as f64;
    let mut ranks: HashMap<Url, f64> = pages.iter().cloned().map(|url| (url, initial)).collect();

    for _ in 0..iterations {
        let dangling_rank: f64 = outgoing
            .iter()
            .filter(|(_, links)| links.is_empty())
            .map(|(url, _)| ranks.get(url).copied().unwrap_or(0.0))
            .sum();

        let mut next = HashMap::new();
        for page in &pages {
            let mut score = dangling_rank / page_count as f64;
            if let Some(backlinks) = incoming.get(page) {
                for backlink in backlinks {
                    let out_count = outgoing.get(backlink).map(Vec::len).unwrap_or(1).max(1);
                    score += ranks.get(backlink).copied().unwrap_or(0.0) / out_count as f64;
                }
            }
            next.insert(
                page.clone(),
                (1.0 - damping) / page_count as f64 + damping * score,
            );
        }
        ranks = next;
    }

    // Scaling around 1.0 keeps PageRank readable in API diagnostics.
    ranks
        .into_iter()
        .map(|(url, rank)| (url, rank * page_count as f64))
        .collect()
}
