use std::{collections::HashMap, path::Path};

use anyhow::{Context, Result};
use async_trait::async_trait;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use walkdir::WalkDir;

use crate::{
    config::AwsSettings,
    models::{CrawledPage, IndexBuildRecord, IndexBuildStatus, IndexManifest},
    storage::IndexStorage,
};

pub struct AwsIndexStore {
    s3: S3Client,
    dynamodb: DynamoClient,
    crawler_pages_table: String,
    index_builds_table: String,
    raw_html_bucket: String,
    index_bucket: String,
}

impl AwsIndexStore {
    pub async fn from_settings(settings: AwsSettings) -> Result<Self> {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        Ok(Self {
            s3: S3Client::new(&config),
            dynamodb: DynamoClient::new(&config),
            crawler_pages_table: settings.crawler_pages_table,
            index_builds_table: settings.index_builds_table,
            raw_html_bucket: settings.raw_html_bucket,
            index_bucket: settings.index_bucket,
        })
    }
}

#[async_trait]
impl IndexStorage for AwsIndexStore {
    async fn put_build(&self, record: IndexBuildRecord) -> Result<()> {
        self.dynamodb
            .put_item()
            .table_name(&self.index_builds_table)
            .set_item(Some(build_item(record)))
            .send()
            .await?;
        Ok(())
    }

    async fn get_build(&self, index_build_id: &str) -> Result<Option<IndexBuildRecord>> {
        let item = self
            .dynamodb
            .get_item()
            .table_name(&self.index_builds_table)
            .key(
                "index_build_id",
                AttributeValue::S(index_build_id.to_string()),
            )
            .send()
            .await?
            .item;

        item.map(build_from_item).transpose()
    }

    async fn list_crawl_pages(&self, crawl_id: &str) -> Result<Vec<CrawledPage>> {
        let mut pages = Vec::new();
        let mut exclusive_start_key = None;

        loop {
            let response = self
                .dynamodb
                .query()
                .table_name(&self.crawler_pages_table)
                .key_condition_expression("crawl_id = :crawl_id")
                .expression_attribute_values(":crawl_id", AttributeValue::S(crawl_id.to_string()))
                .set_exclusive_start_key(exclusive_start_key)
                .send()
                .await?;

            for item in response.items.unwrap_or_default() {
                pages.push(page_from_item(item)?);
            }

            exclusive_start_key = response.last_evaluated_key;
            if exclusive_start_key.is_none() {
                break;
            }
        }

        Ok(pages)
    }

    async fn get_raw_html(&self, s3_key: &str) -> Result<String> {
        let object = self
            .s3
            .get_object()
            .bucket(&self.raw_html_bucket)
            .key(s3_key)
            .send()
            .await?;
        let bytes = object.body.collect().await?.into_bytes();
        Ok(String::from_utf8(bytes.to_vec()).context("raw HTML object is not valid UTF-8")?)
    }

    async fn upload_index_dir(&self, s3_prefix: &str, dir: &Path) -> Result<()> {
        for entry in WalkDir::new(dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
        {
            let path = entry.path();
            let relative = path.strip_prefix(dir)?;
            let relative_key = relative.to_string_lossy().replace('\\', "/");
            let key = format!("{s3_prefix}/{relative_key}");
            let bytes = tokio::fs::read(path).await?;

            self.s3
                .put_object()
                .bucket(&self.index_bucket)
                .key(key)
                .content_type("application/octet-stream")
                .body(ByteStream::from(bytes))
                .send()
                .await?;
        }
        Ok(())
    }

    async fn put_manifest(&self, s3_key: &str, manifest: &IndexManifest) -> Result<()> {
        let body = serde_json::to_vec_pretty(manifest)?;
        self.s3
            .put_object()
            .bucket(&self.index_bucket)
            .key(s3_key)
            .content_type("application/json")
            .body(ByteStream::from(body))
            .send()
            .await?;
        Ok(())
    }
}

fn build_item(record: IndexBuildRecord) -> HashMap<String, AttributeValue> {
    let mut item = HashMap::new();
    item.insert("index_build_id".to_string(), string(record.index_build_id));
    item.insert("crawl_id".to_string(), string(record.crawl_id));
    item.insert(
        "status".to_string(),
        string(build_status_to_str(record.status)),
    );
    item.insert(
        "created_at".to_string(),
        string(record.created_at.to_rfc3339()),
    );
    item.insert("pages_seen".to_string(), number(record.pages_seen));
    item.insert("pages_indexed".to_string(), number(record.pages_indexed));
    item.insert(
        "pages_skipped_non_english".to_string(),
        number(record.pages_skipped_non_english),
    );
    item.insert(
        "pages_skipped_short".to_string(),
        number(record.pages_skipped_short),
    );
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
    insert_opt_string(&mut item, "index_s3_prefix", record.index_s3_prefix);
    insert_opt_string(&mut item, "manifest_s3_key", record.manifest_s3_key);
    insert_opt_string(&mut item, "error", record.error);
    item
}

fn build_from_item(mut item: HashMap<String, AttributeValue>) -> Result<IndexBuildRecord> {
    Ok(IndexBuildRecord {
        index_build_id: take_s(&mut item, "index_build_id")?,
        crawl_id: take_s(&mut item, "crawl_id")?,
        status: parse_build_status(&take_s(&mut item, "status")?)?,
        created_at: take_time(&mut item, "created_at")?,
        started_at: take_opt_time(&mut item, "started_at")?,
        finished_at: take_opt_time(&mut item, "finished_at")?,
        index_s3_prefix: take_opt_s(&mut item, "index_s3_prefix")?,
        manifest_s3_key: take_opt_s(&mut item, "manifest_s3_key")?,
        pages_seen: take_n(&mut item, "pages_seen")?,
        pages_indexed: take_n(&mut item, "pages_indexed")?,
        pages_skipped_non_english: take_n(&mut item, "pages_skipped_non_english")?,
        pages_skipped_short: take_n(&mut item, "pages_skipped_short")?,
        pages_failed: take_n(&mut item, "pages_failed")?,
        error: take_opt_s(&mut item, "error")?,
    })
}

fn page_from_item(mut item: HashMap<String, AttributeValue>) -> Result<CrawledPage> {
    Ok(CrawledPage {
        crawl_id: take_s(&mut item, "crawl_id")?,
        url_hash: take_s(&mut item, "url_hash")?,
        url: take_s(&mut item, "url")?,
        status: take_s(&mut item, "status")?,
        http_status: take_opt_n(&mut item, "http_status")?,
        content_type: take_opt_s(&mut item, "content_type")?,
        title: take_opt_s(&mut item, "title")?,
        s3_key: take_opt_s(&mut item, "s3_key")?,
        content_hash: take_opt_s(&mut item, "content_hash")?,
        links: take_string_list(&mut item, "links")?,
        word_count: take_opt_n(&mut item, "word_count")?,
        error: take_opt_s(&mut item, "error")?,
    })
}

fn string(value: impl Into<String>) -> AttributeValue {
    AttributeValue::S(value.into())
}

fn number(value: impl ToString) -> AttributeValue {
    AttributeValue::N(value.to_string())
}

fn insert_opt_string(item: &mut HashMap<String, AttributeValue>, key: &str, value: Option<String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        item.insert(key.to_string(), string(value));
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

fn take_opt_n<T>(item: &mut HashMap<String, AttributeValue>, key: &str) -> Result<Option<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match item.remove(key) {
        Some(AttributeValue::N(value)) => Ok(Some(value.parse()?)),
        Some(_) => anyhow::bail!("DynamoDB field {key} is not a number"),
        None => Ok(None),
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

fn build_status_to_str(status: IndexBuildStatus) -> &'static str {
    match status {
        IndexBuildStatus::Queued => "queued",
        IndexBuildStatus::Running => "running",
        IndexBuildStatus::Completed => "completed",
        IndexBuildStatus::Failed => "failed",
    }
}

fn parse_build_status(value: &str) -> Result<IndexBuildStatus> {
    match value {
        "queued" => Ok(IndexBuildStatus::Queued),
        "running" => Ok(IndexBuildStatus::Running),
        "completed" => Ok(IndexBuildStatus::Completed),
        "failed" => Ok(IndexBuildStatus::Failed),
        _ => anyhow::bail!("unknown index build status {value}"),
    }
}
