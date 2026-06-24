use anyhow::{Context, Result};
use arxivist_core::CrawlRecord;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

// read the records of the crawled pages. i guess creates a json object we can work with in rust.
pub fn read_records(path: &Path) -> Result<Vec<CrawlRecord>> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        records.push(serde_json::from_str(&line)?);
    }

    Ok(records)
}
