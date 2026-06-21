use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};

use crate::{
    config::AwsSettings,
    ids::raw_html_s3_key,
    models::{AssetRecord, CrawlRecord, CrawlStatus, PageRecord, PageStatus},
    storage::CrawlStorage,
};

pub struct AwsStore {
    s3: S3Client,
    dynamodb: DynamoClient,
    bucket: String,
    crawls_table: String,
    pages_table: String,
}

impl AwsStore {
    pub async fn from_settings(settings: AwsSettings) -> Result<Self> {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        Ok(Self {
            s3: S3Client::new(&config), // so each crawl we start fresh with a new bucket and metadata table?
            dynamodb: DynamoClient::new(&config),
            bucket: settings.s3_bucket,
            crawls_table: settings.crawls_table,
            pages_table: settings.pages_table,
        })
    }
}

#[async_trait]
impl CrawlStorage for AwsStore {
    // starts a new crawl
    async fn put_crawl(&self, record: CrawlRecord) -> Result<()> {
        self.dynamodb
            .put_item()
            .table_name(&self.crawls_table)
            .set_item(Some(crawl_item(record)?))
            .send()
            .await?;
        Ok(())
    }
    // gets information about the crawl id such as max pages crawled?
    async fn get_crawl(&self, crawl_id: &str) -> Result<Option<CrawlRecord>> {
        let item = self
            .dynamodb
            .get_item()
            .table_name(&self.crawls_table)
            .key("crawl_id", AttributeValue::S(crawl_id.to_string()))
            .send()
            .await?
            .item;

        item.map(crawl_record_from_item).transpose()
    }
    // send page metadata to dynamo db
    async fn put_page(&self, record: PageRecord) -> Result<()> {
        self.dynamodb
            .put_item()
            .table_name(&self.pages_table)
            .set_item(Some(page_item(record)?))
            .send()
            .await?;
        Ok(())
    }
    // send raw html to s3
    async fn put_raw_html(&self, crawl_id: &str, url_hash: &str, html: String) -> Result<String> {
        let key = raw_html_s3_key(crawl_id, url_hash);
        self.s3
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .content_type("text/html; charset=utf-8")
            .body(ByteStream::from(html.into_bytes()))
            .send()
            .await?;
        Ok(key)
    }
}
// send crawl info to a hashmap
fn crawl_item(record: CrawlRecord) -> Result<HashMap<String, AttributeValue>> {
    let mut item = HashMap::new();
    item.insert("crawl_id".to_string(), string(record.crawl_id));
    item.insert("status".to_string(), string(status_to_str(record.status)));
    item.insert("seed_urls".to_string(), string_list(record.seed_urls));
    item.insert("max_pages".to_string(), number(record.max_pages));
    item.insert("depth_limit".to_string(), number(record.depth_limit));
    item.insert(
        "created_at".to_string(),
        string(record.created_at.to_rfc3339()),
    );
    item.insert("pages_fetched".to_string(), number(record.pages_fetched));
    item.insert("pages_failed".to_string(), number(record.pages_failed));
    insert_opt_string(
        &mut item,
        "started_at",
        record.started_at.map(|t| t.to_rfc3339()),
    );
    insert_opt_string(
        &mut item,
        "finished_at",
        record.finished_at.map(|t| t.to_rfc3339()),
    );
    insert_opt_string(&mut item, "error", record.error);
    Ok(item)
}
// send page information to a hashmap
fn page_item(record: PageRecord) -> Result<HashMap<String, AttributeValue>> {
    let mut item = HashMap::new();
    item.insert("crawl_id".to_string(), string(record.crawl_id));
    item.insert("url_hash".to_string(), string(record.url_hash));
    item.insert("url".to_string(), string(record.url));
    item.insert(
        "status".to_string(),
        string(page_status_to_str(record.status)),
    );
    item.insert("links".to_string(), string_list(record.links));
    item.insert(
        "assets_json".to_string(),
        string(serde_json::to_string(&record.assets)?),
    );
    item.insert("word_count".to_string(), number(record.word_count));
    item.insert(
        "fetched_at".to_string(),
        string(record.fetched_at.to_rfc3339()),
    );
    insert_opt_number(&mut item, "http_status", record.http_status);
    insert_opt_string(&mut item, "content_type", record.content_type);
    insert_opt_string(&mut item, "title", record.title);
    insert_opt_string(&mut item, "s3_key", record.s3_key);
    insert_opt_string(&mut item, "content_hash", record.content_hash);
    insert_opt_string(&mut item, "text_preview", record.text_preview);
    insert_opt_string(&mut item, "error", record.error);
    Ok(item)
}
// return info a a crawl as a struct
fn crawl_record_from_item(mut item: HashMap<String, AttributeValue>) -> Result<CrawlRecord> {
    Ok(CrawlRecord {
        crawl_id: take_s(&mut item, "crawl_id")?,
        status: parse_status(&take_s(&mut item, "status")?)?,
        seed_urls: take_string_list(&mut item, "seed_urls")?,
        max_pages: take_n(&mut item, "max_pages")?,
        depth_limit: take_n(&mut item, "depth_limit")?,
        created_at: take_time(&mut item, "created_at")?,
        started_at: take_opt_time(&mut item, "started_at")?,
        finished_at: take_opt_time(&mut item, "finished_at")?,
        pages_fetched: take_n(&mut item, "pages_fetched")?,
        pages_failed: take_n(&mut item, "pages_failed")?,
        error: take_opt_s(&mut item, "error")?,
    })
}
// some functions for <type> to string conversions
fn string(value: impl Into<String>) -> AttributeValue {
    AttributeValue::S(value.into())
}

fn number(value: impl ToString) -> AttributeValue {
    AttributeValue::N(value.to_string())
}

fn string_list(values: Vec<String>) -> AttributeValue {
    AttributeValue::L(values.into_iter().map(AttributeValue::S).collect())
}

fn insert_opt_string(item: &mut HashMap<String, AttributeValue>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        item.insert(key.to_string(), string(value));
    }
}

fn insert_opt_number<T: ToString>(
    item: &mut HashMap<String, AttributeValue>,
    key: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        item.insert(key.to_string(), number(value));
    }
}

fn take_s(item: &mut HashMap<String, AttributeValue>, key: &str) -> Result<String> {
    take_opt_s(item, key)?.ok_or_else(|| anyhow::anyhow!("missing DynamoDB string field {key}"))
}

fn take_opt_s(item: &mut HashMap<String, AttributeValue>, key: &str) -> Result<Option<String>> {
    match item.remove(key) {
        Some(AttributeValue::S(value)) => Ok(Some(value)),
        Some(_) => anyhow::bail!("DynamoDB field {key} is not a string"),
        None => Ok(None),
    }
}

fn take_n<T>(item: &mut HashMap<String, AttributeValue>, key: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match item.remove(key) {
        Some(AttributeValue::N(value)) => Ok(value.parse()?),
        Some(_) => anyhow::bail!("DynamoDB field {key} is not a number"),
        None => anyhow::bail!("missing DynamoDB number field {key}"),
    }
}

fn take_string_list(item: &mut HashMap<String, AttributeValue>, key: &str) -> Result<Vec<String>> {
    match item.remove(key) {
        Some(AttributeValue::L(values)) => values
            .into_iter()
            .map(|value| match value {
                AttributeValue::S(value) => Ok(value),
                _ => anyhow::bail!("DynamoDB field {key} contains a non-string value"),
            })
            .collect(),
        Some(_) => anyhow::bail!("DynamoDB field {key} is not a list"),
        None => Ok(Vec::new()),
    }
}

fn take_time(
    item: &mut HashMap<String, AttributeValue>,
    key: &str,
) -> Result<chrono::DateTime<chrono::Utc>> {
    Ok(take_s(item, key)?.parse::<chrono::DateTime<chrono::Utc>>()?)
}

fn take_opt_time(
    item: &mut HashMap<String, AttributeValue>,
    key: &str,
) -> Result<Option<chrono::DateTime<chrono::Utc>>> {
    take_opt_s(item, key)?
        .map(|value| value.parse::<chrono::DateTime<chrono::Utc>>())
        .transpose()
        .map_err(Into::into)
}

fn status_to_str(status: CrawlStatus) -> &'static str {
    match status {
        CrawlStatus::Queued => "queued",
        CrawlStatus::Running => "running",
        CrawlStatus::Completed => "completed",
        CrawlStatus::Failed => "failed",
    }
}

fn parse_status(value: &str) -> Result<CrawlStatus> {
    match value {
        "queued" => Ok(CrawlStatus::Queued),
        "running" => Ok(CrawlStatus::Running),
        "completed" => Ok(CrawlStatus::Completed),
        "failed" => Ok(CrawlStatus::Failed),
        _ => anyhow::bail!("unknown crawl status {value}"),
    }
}

fn page_status_to_str(status: PageStatus) -> &'static str {
    match status {
        PageStatus::Fetched => "fetched",
        PageStatus::Failed => "failed",
        PageStatus::Skipped => "skipped",
    }
}

#[allow(dead_code)]
fn _assets_from_json(value: &str) -> Result<Vec<AssetRecord>> {
    Ok(serde_json::from_str(value)?)
}
