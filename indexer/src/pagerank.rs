use std::collections::{HashMap, HashSet};

use crate::models::{IndexedPage, PageRankEntry};

pub fn compute_page_rank(
    pages: &[IndexedPage],
    damping: f64,
    iterations: usize,
) -> HashMap<String, f64> {
    let page_count = pages.len();
    if page_count == 0 {
        return HashMap::new();
    }

    let n = page_count as f64;
    let init_rank = 1.0 / n;
    let url_to_hash = pages
        .iter()
        .map(|page| (page.url.as_str(), page.url_hash.as_str()))
        .collect::<HashMap<_, _>>();
    let page_ids = pages
        .iter()
        .map(|page| page.url_hash.clone())
        .collect::<HashSet<_>>();

    let mut outlinks = HashMap::<String, Vec<String>>::new();
    let mut backlinks = HashMap::<String, Vec<String>>::new();

    for page in pages {
        let mut targets = page
            .links
            .iter()
            .filter_map(|link| url_to_hash.get(link.as_str()).copied())
            .filter(|target| *target != page.url_hash)
            .filter(|target| page_ids.contains(*target))
            .map(str::to_string)
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();

        for target in &targets {
            backlinks
                .entry(target.clone())
                .or_default()
                .push(page.url_hash.clone());
        }
        outlinks.insert(page.url_hash.clone(), targets);
    }

    let mut ranks = page_ids
        .iter()
        .map(|page_id| (page_id.clone(), init_rank))
        .collect::<HashMap<_, _>>();

    for _ in 0..iterations {
        let dangling_sum = page_ids
            .iter()
            .filter(|page_id| outlinks.get(*page_id).is_none_or(Vec::is_empty))
            .map(|page_id| ranks.get(page_id).copied().unwrap_or(0.0))
            .sum::<f64>();

        let mut next = HashMap::with_capacity(page_count);

        for page_id in &page_ids {
            let incoming = backlinks
                .get(page_id)
                .into_iter()
                .flatten()
                .map(|source_id| {
                    let source_rank = ranks.get(source_id).copied().unwrap_or(0.0);
                    let out_count = outlinks.get(source_id).map_or(0, Vec::len);
                    if out_count == 0 {
                        0.0
                    } else {
                        source_rank / out_count as f64
                    }
                })
                .sum::<f64>();

            let rank = (1.0 - damping) / n + damping * (incoming + dangling_sum / n);
            next.insert(page_id.clone(), rank);
        }

        ranks = next;
    }

    ranks
        .into_iter()
        .map(|(page_id, rank)| (page_id, rank * n))
        .collect()
}

pub fn page_rank_entries(pages: &[IndexedPage]) -> Vec<PageRankEntry> {
    let mut entries = pages
        .iter()
        .map(|page| PageRankEntry {
            url_hash: page.url_hash.clone(),
            url: page.url.clone(),
            page_rank: page.page_rank,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.url_hash.cmp(&b.url_hash));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str, links: Vec<&str>) -> IndexedPage {
        IndexedPage {
            crawl_id: "crawl".to_string(),
            url_hash: crate::ids::url_hash(url),
            url: url.to_string(),
            title: None,
            body: "body".to_string(),
            text_preview: "body".to_string(),
            s3_key: "raw.html".to_string(),
            content_hash: None,
            links: links.into_iter().map(str::to_string).collect(),
            word_count: 1,
            page_rank: 0.0,
        }
    }

    #[test]
    fn linked_page_gets_higher_rank() {
        let pages = vec![
            page("https://example.com/a", vec!["https://example.com/b"]),
            page("https://example.com/b", Vec::new()),
        ];

        let ranks = compute_page_rank(&pages, 0.85, 20);
        let a = ranks
            .get(&crate::ids::url_hash("https://example.com/a"))
            .unwrap();
        let b = ranks
            .get(&crate::ids::url_hash("https://example.com/b"))
            .unwrap();

        assert!(b > a);
    }
}
