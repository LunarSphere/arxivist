// all of our code for handling AWS
use crate::{
    args::Args,
    filters, record, spider_client,
    types::{PageSnapshot, QueueItem},
};
use anyhow::{Context, Result, anyhow};
use arxivist_core::{CrawlOutcome, CrawlRecord, CrawlSkipReason, content_hash};
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use aws_sdk_sqs::Client as SqsClient;
use std::collections::{HashMap, HashSet};
use tracing::{info, warn};
use url::Url;

struct AwsStores {
    s3: S3Client,
    dynamodb: DynamoClient,
    sqs: SqsClient,
    data_bucket: String,      // bucket we keep page content in
    pages_table: String,      // page metadata table
    crawl_urls_table: String, // track urls we crawl
    crawl_queue_url: String,  //urls queued w/ SQS
}

// run crawler on aws
pub async fn run(args: Args) -> Result<()> {
    let stores = AwsStores::from_args(&args).await?; //
    for seed in &args.seeds {
        let item = QueueItem {
            url: seed.clone(),
            source_seed: seed.clone(),
            referrer: None,
            depth: 0,
        };
        stores.enqueue_if_new(&item).await?;
    }

    let mut processed = 0usize;
    let mut stored = 0usize;
    let mut empty_receives = 0usize;
    let mut bad_hosts: HashMap<String, usize> = HashMap::new();
    let mut suppressed_hosts = HashSet::new();

    while processed < args.max_pages && empty_receives < args.empty_receive_limit {
        let Some(message) = stores.receive_one().await? else {
            empty_receives += 1;
            continue;
        };
        empty_receives = 0;

        let Some(body) = message.body() else {
            if let Some(handle) = message.receipt_handle() {
                stores.delete_message(handle).await?;
            }
            continue;
        };
        let item: QueueItem = serde_json::from_str(body).context("decode crawl queue item")?;
        let record = if item
            .url
            .host_str()
            .is_some_and(|host| suppressed_hosts.contains(host))
        {
            record::diagnostic(
                &item,
                CrawlOutcome::HostSuppressed,
                Some(CrawlSkipReason::BadHostThreshold),
            )
        } else {
            crawl_and_store(&args, &stores, &item).await?
        };

        if filters::should_penalize(record.skip_reason) {
            if let Some(host) = item.url.host_str().map(str::to_owned) {
                let count = bad_hosts.entry(host.clone()).or_default();
                *count += 1;
                if *count >= args.bad_host_threshold {
                    suppressed_hosts.insert(host);
                }
            }
        }

        if record.outcome == CrawlOutcome::Stored {
            stored += 1;
            if item.depth < args.max_depth {
                for link in &record.links {
                    stores
                        .enqueue_if_new(&QueueItem {
                            url: link.clone(),
                            source_seed: item.source_seed.clone(),
                            referrer: record.final_url.clone(),
                            depth: item.depth + 1,
                        })
                        .await?;
                }
            }
        }

        stores.put_record(&record).await?;
        if let Some(handle) = message.receipt_handle() {
            stores.delete_message(handle).await?;
        }

        processed += 1;
        info!(
            requested_url = %record.requested_url,
            outcome = ?record.outcome,
            stored,
            processed,
            "processed aws crawl record"
        );
    }

    info!(stored, processed, "aws crawl complete");
    Ok(())
}

async fn crawl_and_store(args: &Args, stores: &AwsStores, item: &QueueItem) -> Result<CrawlRecord> {
    match spider_client::crawl_snapshot(args, item).await {
        Ok(snapshot) => record_from_aws_snapshot(stores, item, snapshot).await,
        Err(record) => Ok(record),
    }
}

async fn record_from_aws_snapshot(
    stores: &AwsStores,
    item: &QueueItem,
    snapshot: PageSnapshot,
) -> Result<CrawlRecord> {
    let hash = content_hash(&snapshot.html);
    let content_path = record::storage_content_path(&hash);
    let html = snapshot.html.clone();
    let record =
        record::from_snapshot_with_content_path(item, snapshot, Some(content_path.clone()));

    if record.outcome == CrawlOutcome::Stored {
        stores
            .s3
            .put_object()
            .bucket(&stores.data_bucket)
            .key(&content_path)
            .content_type("text/html; charset=utf-8")
            .body(ByteStream::from(html.into_bytes()))
            .send()
            .await
            .context("write page snapshot to s3")?;
    }

    Ok(record)
}

impl AwsStores {
    // use aws sdk to connect to relevant clients
    async fn from_args(args: &Args) -> Result<Self> {
        let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
        Ok(Self {
            s3: S3Client::new(&config),
            dynamodb: DynamoClient::new(&config),
            sqs: SqsClient::new(&config),
            data_bucket: required(args.data_bucket.as_deref(), "ARXIVIST_DATA_BUCKET")?,
            pages_table: required(args.pages_table.as_deref(), "ARXIVIST_PAGES_TABLE")?,
            crawl_urls_table: required(
                args.crawl_urls_table.as_deref(),
                "ARXIVIST_CRAWL_URLS_TABLE",
            )?,
            crawl_queue_url: required(args.crawl_queue_url.as_deref(), "ARXIVIST_CRAWL_QUEUE_URL")?,
        })
    }
    // recieve a message from SQS
    async fn receive_one(&self) -> Result<Option<aws_sdk_sqs::types::Message>> {
        let output = self
            .sqs
            .receive_message()
            .queue_url(&self.crawl_queue_url)
            .max_number_of_messages(1)
            .wait_time_seconds(10)
            .send()
            .await
            .context("receive crawl message")?;
        Ok(output.messages().first().cloned())
    }
    // delete a message from SQS
    async fn delete_message(&self, receipt_handle: &str) -> Result<()> {
        self.sqs
            .delete_message()
            .queue_url(&self.crawl_queue_url)
            .receipt_handle(receipt_handle)
            .send()
            .await
            .context("delete crawl message")?;
        Ok(())
    }
    // Add to SQQ queue if url is new
    async fn enqueue_if_new(&self, item: &QueueItem) -> Result<()> {
        let url_hash = content_hash(item.url.as_str());
        let put = self
            .dynamodb
            .put_item()
            .table_name(&self.crawl_urls_table)
            .item("url_hash", AttributeValue::S(url_hash))
            .item("url", AttributeValue::S(item.url.as_str().to_owned()))
            .item("status", AttributeValue::S("queued".to_owned()))
            .item("updated_at", AttributeValue::S(now_ms().to_string()))
            .condition_expression("attribute_not_exists(url_hash)")
            .send()
            .await;

        if let Err(error) = put {
            let message = error.to_string();
            if message.contains("ConditionalCheckFailed") {
                return Ok(());
            }
            return Err(error).context("deduplicate crawl url");
        }

        self.sqs
            .send_message()
            .queue_url(&self.crawl_queue_url)
            .message_body(serde_json::to_string(item)?)
            .send()
            .await
            .context("enqueue crawl item")?;
        Ok(())
    }

    // write crawl record to dynamodb
    async fn put_record(&self, record: &CrawlRecord) -> Result<()> {
        let url_hash = content_hash(record.requested_url.as_str());
        let final_url = record
            .final_url
            .as_ref()
            .map(Url::as_str)
            .unwrap_or_default()
            .to_owned();

        self.dynamodb
            .put_item()
            .table_name(&self.pages_table)
            .item("url_hash", AttributeValue::S(url_hash.clone()))
            .item(
                "requested_url",
                AttributeValue::S(record.requested_url.as_str().to_owned()),
            )
            .item("final_url", AttributeValue::S(final_url))
            .item("outcome", AttributeValue::S(outcome_status(record.outcome)))
            .item(
                "fetched_at_ms",
                AttributeValue::N(record.fetched_at_ms.to_string()),
            )
            .item(
                "record_json",
                AttributeValue::S(serde_json::to_string(record)?),
            )
            .send()
            .await
            .context("write crawl record to dynamodb")?;

        if let Err(error) = self
            .dynamodb
            .update_item()
            .table_name(&self.crawl_urls_table)
            .key("url_hash", AttributeValue::S(url_hash))
            .update_expression("SET #status = :status, updated_at = :updated_at")
            .expression_attribute_names("#status", "status")
            .expression_attribute_values(
                ":status",
                AttributeValue::S(outcome_status(record.outcome)),
            )
            .expression_attribute_values(
                ":updated_at",
                AttributeValue::S(record.fetched_at_ms.to_string()),
            )
            .send()
            .await
        {
            warn!(?error, "failed to update crawl url status");
        }

        Ok(())
    }
}

fn required(value: Option<&str>, name: &str) -> Result<String> {
    value
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("{name} is required when --storage aws is used"))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn outcome_status(outcome: CrawlOutcome) -> String {
    match outcome {
        CrawlOutcome::Stored => "stored",
        CrawlOutcome::Skipped => "skipped",
        CrawlOutcome::RobotsBlocked => "robots_blocked",
        CrawlOutcome::HostSuppressed => "host_suppressed",
        CrawlOutcome::FetchFailed => "fetch_failed",
    }
    .to_owned()
}
