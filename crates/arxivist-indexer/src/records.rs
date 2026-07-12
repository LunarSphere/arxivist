use anyhow::{Context, Result};
use arxivist_core::{CrawlExtractedPayload, CrawlOutcome, CrawlRecord};
use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
};

// read the records of the crawled pages. converts json info into a vector of crawl record structs
pub fn read_records(path: &Path) -> Result<Vec<CrawlRecord>> {
    let file = fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut records = Vec::new();
    let crawl_dir = path.parent().unwrap_or_else(|| Path::new("."));

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut record: CrawlRecord = serde_json::from_str(&line).context("decode crawl record")?;
        hydrate_local_payload(crawl_dir, &mut record)?;
        records.push(record);
    }

    Ok(records)
}

fn hydrate_local_payload(crawl_dir: &Path, record: &mut CrawlRecord) -> Result<()> {
    if record.outcome != CrawlOutcome::Stored || !record.extracted_text.trim().is_empty() {
        return Ok(());
    }

    let Some(payload_path) = record.extracted_payload_path.as_deref() else {
        return Ok(());
    };
    // Local crawl metadata can point at extracted payload files. Hydrating here
    // keeps index construction independent from where crawl text is stored.
    let payload_path = crawl_dir.join(payload_path);
    let payload: CrawlExtractedPayload = serde_json::from_slice(
        &fs::read(&payload_path)
            .with_context(|| format!("read extracted payload {}", payload_path.display()))?,
    )
    .with_context(|| format!("decode extracted payload {}", payload_path.display()))?;
    record.extracted_text = payload.extracted_text;
    record.links = payload.links;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arxivist_core::CrawlSkipReason;
    use std::io::Write;
    use url::Url;

    #[test]
    fn read_records_hydrates_local_extracted_payloads() {
        let dir = temp_dir("arxivist-records-payload");
        fs::create_dir_all(dir.join("extracted")).unwrap();
        let payload_path = dir.join("extracted/hash.json");
        fs::write(
            &payload_path,
            serde_json::to_vec(&CrawlExtractedPayload {
                extracted_text: "payload text that should be indexed".to_owned(),
                links: vec![Url::parse("https://example.com/next").unwrap()],
            })
            .unwrap(),
        )
        .unwrap();
        let mut record = record(
            Url::parse("https://example.com/page").unwrap(),
            CrawlOutcome::Stored,
            None,
            "",
        );
        record.extracted_payload_path = Some("extracted/hash.json".to_owned());

        write_jsonl_record(&dir.join("pages.jsonl"), &record);

        let records = read_records(&dir.join("pages.jsonl")).unwrap();

        assert_eq!(
            records[0].extracted_text,
            "payload text that should be indexed"
        );
        assert_eq!(records[0].links.len(), 1);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_records_keeps_inline_extracted_text() {
        let dir = temp_dir("arxivist-records-inline");
        fs::create_dir_all(&dir).unwrap();
        let mut record = record(
            Url::parse("https://example.com/page").unwrap(),
            CrawlOutcome::Stored,
            None,
            "inline text stays authoritative",
        );
        record.extracted_payload_path = Some("extracted/missing.json".to_owned());

        write_jsonl_record(&dir.join("pages.jsonl"), &record);

        let records = read_records(&dir.join("pages.jsonl")).unwrap();

        assert_eq!(records[0].extracted_text, "inline text stays authoritative");

        let _ = fs::remove_dir_all(dir);
    }

    fn write_jsonl_record(path: &Path, record: &CrawlRecord) {
        let mut file = fs::File::create(path).unwrap();
        serde_json::to_writer(&mut file, record).unwrap();
        writeln!(file).unwrap();
    }

    fn record(
        url: Url,
        outcome: CrawlOutcome,
        skip_reason: Option<CrawlSkipReason>,
        text: &str,
    ) -> CrawlRecord {
        CrawlRecord {
            schema_version: 2,
            requested_url: url.clone(),
            final_url: Some(url.clone()),
            source_seed: url,
            referrer: None,
            depth: 0,
            outcome,
            skip_reason,
            title: None,
            status: Some(200),
            content_type: Some("text/html".to_owned()),
            content_length: Some(10),
            content_hash: None,
            content_path: None,
            extracted_payload_path: None,
            extracted_text: text.to_owned(),
            links: Vec::new(),
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
