use crate::{
    Args, index,
    pagerank::{self, PageLinks},
    required,
};
use anyhow::{Context, Result, anyhow};
use arxivist_core::{CrawlExtractedPayload, CrawlOutcome, CrawlRecord};
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};
use tracing::{info, warn};
use url::Url;

pub async fn run(args: &Args) -> Result<()> {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamodb = DynamoClient::new(&config);
    let s3 = S3Client::new(&config);
    let bucket = required(args.data_bucket.as_deref(), "ARXIVIST_DATA_BUCKET")?;
    let pages_table = required(args.pages_table.as_deref(), "ARXIVIST_PAGES_TABLE")?;

    let records = read_index_inputs(&dynamodb, &s3, &bucket, &pages_table).await?;
    let AwsReadResult {
        input_spool_path,
        input_count,
        links,
        pages,
    } = records;
    let page_ranks = pagerank::compute_page_rank_from_links(&pages, &links, 0.85, 20);
    drop(links);
    drop(pages);

    let input_reader = SpoolInputReader::open(&input_spool_path)?;
    let input_errors = input_reader.errors();
    let search_index = index::build_index_from_iter(input_reader, page_ranks);
    if let Some(error) = input_errors.borrow_mut().take() {
        return Err(error);
    }
    let _ = fs::remove_file(&input_spool_path);

    let docs = search_index.documents.len();
    let terms = search_index.terms.len();
    let version = index::version_from_time();
    let index_dir = temp_dir("arxivist-index");
    fs::create_dir_all(&index_dir)
        .with_context(|| format!("create temporary index dir {}", index_dir.display()))?;
    index::write_sharded_index(&search_index, &index_dir, &version)?;
    let index_bytes = directory_size(&index_dir)?;
    drop(search_index);
    let versioned_prefix = format!("indexes/versions/{version}");
    let active_prefix = active_index_prefix(&args.active_index_key);

    put_index_dir(&s3, &bucket, &versioned_prefix, &index_dir).await?;
    put_index_dir(&s3, &bucket, &active_prefix, &index_dir).await?;
    let _ = fs::remove_dir_all(&index_dir);

    info!(
        bucket,
        active_prefix,
        versioned_prefix,
        docs,
        terms,
        input_count,
        index_bytes,
        "wrote aws sharded index"
    );
    Ok(())
}

struct AwsReadResult {
    input_spool_path: PathBuf,
    input_count: usize,
    links: Vec<PageLinks>,
    pages: HashSet<Url>,
}

#[derive(Serialize, Deserialize)]
struct SpoolIndexInput {
    final_url: Url,
    title: Option<String>,
    extracted_text: String,
}

struct PageMetadata {
    requested_url: Option<Url>,
    final_url: Option<Url>,
    outcome: CrawlOutcome,
    title: Option<String>,
    extracted_payload_path: Option<String>,
}

async fn read_index_inputs(
    dynamodb: &DynamoClient,
    s3: &S3Client,
    bucket: &str,
    table_name: &str,
) -> Result<AwsReadResult> {
    let input_spool_path = temp_path("arxivist-inputs", "jsonl");
    let mut input_spool = BufWriter::new(
        File::create(&input_spool_path)
            .with_context(|| format!("create input spool {}", input_spool_path.display()))?,
    );
    let mut input_count = 0usize;
    let mut links = Vec::new();
    let mut pages = HashSet::new();
    let mut start_key = None;
    let mut scanned_items = 0usize;
    let mut stored_pages = 0usize;
    let mut legacy_records = 0usize;
    let mut payload_bytes = 0usize;
    let mut skipped_stored_items = 0usize;

    loop {
        let output = dynamodb
            .scan()
            .table_name(table_name)
            .projection_expression(
                "#requested_url, #final_url, #outcome, #title, #extracted_payload_path, #record_json",
            )
            .expression_attribute_names("#requested_url", "requested_url")
            .expression_attribute_names("#final_url", "final_url")
            .expression_attribute_names("#outcome", "outcome")
            .expression_attribute_names("#title", "title")
            .expression_attribute_names("#extracted_payload_path", "extracted_payload_path")
            .expression_attribute_names("#record_json", "record_json")
            .set_exclusive_start_key(start_key)
            .send()
            .await
            .context("scan crawl records table")?;

        for item in output.items() {
            scanned_items += 1;

            if let Some(metadata) = metadata_from_item(item)? {
                if metadata.outcome != CrawlOutcome::Stored {
                    continue;
                }

                let Some(final_url) = metadata.final_url else {
                    skipped_stored_items += 1;
                    warn!(
                        requested_url = ?metadata.requested_url,
                        "skipping stored page without final url"
                    );
                    continue;
                };
                let Some(payload_path) = metadata.extracted_payload_path else {
                    skipped_stored_items += 1;
                    warn!(%final_url, "skipping stored page without extracted payload path");
                    continue;
                };

                let payload = read_extracted_payload(s3, bucket, &payload_path).await?;
                payload_bytes += payload.extracted_text.len();
                stored_pages += 1;
                pages.insert(final_url.clone());
                links.push(PageLinks {
                    url: final_url.clone(),
                    links: payload.links,
                });
                write_spooled_input(
                    &mut input_spool,
                    &SpoolIndexInput {
                        final_url,
                        title: metadata.title,
                        extracted_text: payload.extracted_text,
                    },
                )?;
                input_count += 1;
                continue;
            }

            if let Some(AttributeValue::S(record_json)) = item.get("record_json") {
                let record: CrawlRecord =
                    serde_json::from_str(record_json).context("decode legacy crawl record")?;
                legacy_records += 1;
                if record.outcome != CrawlOutcome::Stored || record.extracted_text.trim().is_empty()
                {
                    continue;
                }
                let Some(final_url) = record.final_url else {
                    skipped_stored_items += 1;
                    continue;
                };
                payload_bytes += record.extracted_text.len();
                stored_pages += 1;
                pages.insert(final_url.clone());
                links.push(PageLinks {
                    url: final_url.clone(),
                    links: record.links,
                });
                write_spooled_input(
                    &mut input_spool,
                    &SpoolIndexInput {
                        final_url,
                        title: record.title,
                        extracted_text: record.extracted_text,
                    },
                )?;
                input_count += 1;
            }
        }

        start_key = output.last_evaluated_key().cloned();
        if start_key.is_none() {
            break;
        }
    }

    input_spool
        .flush()
        .with_context(|| format!("flush input spool {}", input_spool_path.display()))?;
    drop(input_spool);

    info!(
        scanned_items,
        stored_pages,
        legacy_records,
        skipped_stored_items,
        payload_bytes,
        input_count,
        input_spool_path = %input_spool_path.display(),
        "read crawl records for aws index"
    );

    Ok(AwsReadResult {
        input_spool_path,
        input_count,
        links,
        pages,
    })
}

async fn read_extracted_payload(
    s3: &S3Client,
    bucket: &str,
    key: &str,
) -> Result<CrawlExtractedPayload> {
    let output = s3
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .with_context(|| format!("read extracted crawl payload from s3://{bucket}/{key}"))?;
    let bytes = output.body.collect().await?.into_bytes();
    serde_json::from_slice(&bytes)
        .with_context(|| format!("decode extracted crawl payload from s3://{bucket}/{key}"))
}

fn write_spooled_input(writer: &mut BufWriter<File>, input: &SpoolIndexInput) -> Result<()> {
    serde_json::to_writer(&mut *writer, input).context("encode index input spool record")?;
    writeln!(writer).context("write index input spool newline")?;
    Ok(())
}

struct SpoolInputReader {
    reader: BufReader<File>,
    errors: std::rc::Rc<RefCell<Option<anyhow::Error>>>,
}

impl SpoolInputReader {
    fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            reader: BufReader::new(
                File::open(path).with_context(|| format!("open input spool {}", path.display()))?,
            ),
            errors: std::rc::Rc::new(RefCell::new(None)),
        })
    }

    fn errors(&self) -> std::rc::Rc<RefCell<Option<anyhow::Error>>> {
        self.errors.clone()
    }
}

impl Iterator for SpoolInputReader {
    type Item = index::IndexInput;

    fn next(&mut self) -> Option<Self::Item> {
        if self.errors.borrow().is_some() {
            return None;
        }

        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) if line.trim().is_empty() => continue,
                Ok(_) => {
                    let parsed = serde_json::from_str::<SpoolIndexInput>(&line)
                        .map(|input| index::IndexInput {
                            final_url: input.final_url,
                            title: input.title,
                            extracted_text: input.extracted_text,
                        })
                        .context("decode index input spool record");

                    match parsed {
                        Ok(input) => return Some(input),
                        Err(error) => {
                            *self.errors.borrow_mut() = Some(error);
                            return None;
                        }
                    }
                }
                Err(error) => {
                    *self.errors.borrow_mut() = Some(error.into());
                    return None;
                }
            }
        }
    }
}

fn temp_path(prefix: &str, extension: &str) -> PathBuf {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!(
        "{prefix}-{now_ms}-{}.{}",
        std::process::id(),
        extension
    ))
}

fn temp_dir(prefix: &str) -> PathBuf {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    std::env::temp_dir().join(format!("{prefix}-{now_ms}-{}", std::process::id()))
}

fn metadata_from_item(item: &HashMap<String, AttributeValue>) -> Result<Option<PageMetadata>> {
    let Some(payload_path) = string_attr(item, "extracted_payload_path") else {
        return Ok(None);
    };
    let outcome = string_attr(item, "outcome")
        .map(parse_outcome)
        .transpose()?
        .unwrap_or(CrawlOutcome::Stored);

    Ok(Some(PageMetadata {
        requested_url: parse_optional_url(item, "requested_url")?,
        final_url: parse_optional_url(item, "final_url")?,
        outcome,
        title: string_attr(item, "title").map(str::to_owned),
        extracted_payload_path: Some(payload_path.to_owned()),
    }))
}

fn string_attr<'a>(item: &'a HashMap<String, AttributeValue>, name: &str) -> Option<&'a str> {
    match item.get(name) {
        Some(AttributeValue::S(value)) => Some(value.as_str()),
        _ => None,
    }
}

fn parse_optional_url(item: &HashMap<String, AttributeValue>, name: &str) -> Result<Option<Url>> {
    string_attr(item, name)
        .filter(|value| !value.is_empty())
        .map(Url::parse)
        .transpose()
        .with_context(|| format!("parse {name} url from dynamodb item"))
}

fn parse_outcome(value: &str) -> Result<CrawlOutcome> {
    match value {
        "stored" => Ok(CrawlOutcome::Stored),
        "skipped" => Ok(CrawlOutcome::Skipped),
        "robots_blocked" => Ok(CrawlOutcome::RobotsBlocked),
        "host_suppressed" => Ok(CrawlOutcome::HostSuppressed),
        "fetch_failed" => Ok(CrawlOutcome::FetchFailed),
        other => Err(anyhow!("unknown crawl outcome {other:?}")),
    }
}

async fn put_index_dir(s3: &S3Client, bucket: &str, prefix: &str, dir: &Path) -> Result<()> {
    for path in index_artifact_paths(dir)? {
        let relative = path
            .strip_prefix(dir)
            .with_context(|| format!("make index artifact path relative {}", path.display()))?;
        let key = format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            relative.to_string_lossy().replace('\\', "/")
        );
        let body = ByteStream::from_path(&path)
            .await
            .with_context(|| format!("open temporary index file {}", path.display()))?;

        s3.put_object()
            .bucket(bucket)
            .key(&key)
            .content_type("application/json")
            .body(body)
            .send()
            .await
            .with_context(|| format!("write index artifact to s3://{bucket}/{key}"))?;
    }
    Ok(())
}

fn index_artifact_paths(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    collect_index_artifact_paths(dir, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn collect_index_artifact_paths(dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("read index dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_index_artifact_paths(&path, paths)?;
        } else {
            paths.push(path);
        }
    }
    Ok(())
}

fn directory_size(dir: &Path) -> Result<u64> {
    let mut size = 0u64;
    for path in index_artifact_paths(dir)? {
        size += fs::metadata(&path)
            .with_context(|| format!("stat index artifact {}", path.display()))?
            .len();
    }
    Ok(size)
}

fn active_index_prefix(active_index_key: &str) -> String {
    active_index_key
        .trim_end_matches('/')
        .strip_suffix("/manifest.json")
        .unwrap_or_else(|| active_index_key.trim_end_matches('/'))
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_new_pointer_metadata() {
        let mut item = HashMap::new();
        item.insert(
            "requested_url".to_owned(),
            AttributeValue::S("https://example.com/start".to_owned()),
        );
        item.insert(
            "final_url".to_owned(),
            AttributeValue::S("https://example.com/final".to_owned()),
        );
        item.insert("outcome".to_owned(), AttributeValue::S("stored".to_owned()));
        item.insert("title".to_owned(), AttributeValue::S("Example".to_owned()));
        item.insert(
            "extracted_payload_path".to_owned(),
            AttributeValue::S("crawl/extracted/hash.json".to_owned()),
        );

        let metadata = metadata_from_item(&item).unwrap().unwrap();

        assert_eq!(
            metadata.final_url.unwrap(),
            Url::parse("https://example.com/final").unwrap()
        );
        assert_eq!(
            metadata.extracted_payload_path.unwrap(),
            "crawl/extracted/hash.json"
        );
        assert_eq!(metadata.title.as_deref(), Some("Example"));
    }

    #[test]
    fn ignores_legacy_items_for_pointer_metadata() {
        let mut item = HashMap::new();
        item.insert("record_json".to_owned(), AttributeValue::S("{}".to_owned()));

        assert!(metadata_from_item(&item).unwrap().is_none());
    }
}
