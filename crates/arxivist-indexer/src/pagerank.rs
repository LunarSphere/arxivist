// PageRank prioritizes pages that are linked by other stored pages.
// Links from high-rank pages count more than links from low-rank pages.
use arxivist_core::{CrawlOutcome, CrawlRecord};
use std::collections::{HashMap, HashSet};

use url::Url;

pub struct PageLinks {
    pub url: Url,
    pub links: Vec<Url>,
}

pub fn compute_page_rank(
    records: &[CrawlRecord],
    damping: f64,
    iterations: usize,
) -> HashMap<Url, f64> {
    let pages = records
        .iter()
        .filter(|record| record.outcome == CrawlOutcome::Stored)
        .filter_map(|record| record.final_url.clone())
        .collect::<HashSet<_>>();
    let links = records
        .iter()
        .filter(|record| record.outcome == CrawlOutcome::Stored)
        .filter_map(|record| {
            record.final_url.clone().map(|url| PageLinks {
                url,
                links: record.links.clone(),
            })
        })
        .collect::<Vec<_>>();

    compute_page_rank_from_links(&pages, &links, damping, iterations)
}

pub fn compute_page_rank_from_links(
    pages: &HashSet<Url>,
    links_by_page: &[PageLinks],
    damping: f64,
    iterations: usize,
) -> HashMap<Url, f64> {
    let page_count = pages.len();
    if page_count == 0 {
        return HashMap::new();
    }
    let mut outgoing: HashMap<Url, Vec<Url>> = HashMap::new();
    let mut incoming: HashMap<Url, Vec<Url>> = HashMap::new();

    for page in links_by_page {
        // Keep only outgoing links that point to pages inside this indexed corpus.
        let links: Vec<Url> = page
            .links
            .iter()
            .filter(|link| pages.contains(*link))
            .cloned()
            .collect();
        // Build the reverse lookup: target page -> pages linking to it.
        for link in &links {
            incoming
                .entry(link.clone())
                .or_default()
                .push(page.url.clone());
        }
        // Store the forward lookup: source page -> pages it links to.
        outgoing.insert(page.url.clone(), links);
    }
    // Start with every page having the same rank.
    let initial = 1.0 / page_count as f64;
    let mut ranks: HashMap<Url, f64> = pages.iter().cloned().map(|url| (url, initial)).collect();
    // Recalculate ranks several times so link authority can flow through the graph.
    for _ in 0..iterations {
        // Dangling pages have no outgoing links, so their rank is spread evenly
        // across every page instead of disappearing from the graph.
        let dangling_rank: f64 = outgoing
            .iter()
            .filter(|(_, links)| links.is_empty())
            .map(|(url, _)| ranks.get(url).copied().unwrap_or(0.0))
            .sum();

        let mut next = HashMap::new(); // pagerank scores after this iteration
        for page in pages {
            let mut score = dangling_rank / page_count as f64;
            if let Some(backlinks) = incoming.get(page) {
                for backlink in backlinks {
                    let out_count = outgoing.get(backlink).map(Vec::len).unwrap_or(1).max(1);
                    // A backlink passes along its current rank split across its outgoing links.
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
