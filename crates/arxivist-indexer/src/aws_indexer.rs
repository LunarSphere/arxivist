use crate::{Args, index, pagerank, required};
use anyhow::{Context, Result};
use arxivist_core::CrawlRecord;
use aws_config::BehaviorVersion;
use aws_sdk_dynamodb::{Client as DynamoClient, types::AttributeValue};
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use tracing::info;

pub async fn run(args: &Args) -> Result<()> {
    let config = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let dynamodb = DynamoClient::new(&config);
    let s3 = S3Client::new(&config);
    let bucket = required(args.data_bucket.as_deref(), "ARXIVIST_DATA_BUCKET")?;
    let pages_table = required(args.pages_table.as_deref(), "ARXIVIST_PAGES_TABLE")?;

    let records = read_records(&dynamodb, &pages_table).await?;
    let page_ranks = pagerank::compute_page_rank(&records, 0.85, 20);
    let search_index = index::build_index(records, page_ranks);
    let docs = search_index.documents.len();
    let encoded = serde_json::to_vec(&search_index)?;
    drop(search_index);
    let versioned_key = versioned_index_key();

    put_index(&s3, &bucket, &versioned_key, encoded).await?;
    copy_index(&s3, &bucket, &versioned_key, &args.active_index_key).await?;

    info!(
        bucket,
        active_key = %args.active_index_key,
        versioned_key,
        docs,
        "wrote aws index"
    );
    Ok(())
}
async fn read_records(dynamodb: &DynamoClient, table_name: &str) -> Result<Vec<CrawlRecord>> {
    let mut records = Vec::new();
    let mut start_key = None;

    loop {
        let output = dynamodb
            .scan()
            .table_name(table_name)
            .projection_expression("record_json")
            .set_exclusive_start_key(start_key)
            .send()
            .await
            .context("scan crawl records table")?;

        for item in output.items() {
            if let Some(AttributeValue::S(record_json)) = item.get("record_json") {
                records.push(serde_json::from_str(record_json).context("decode crawl record")?);
            }
        }

        start_key = output.last_evaluated_key().cloned();
        if start_key.is_none() {
            break;
        }
    }

    Ok(records)
}

async fn put_index(s3: &S3Client, bucket: &str, key: &str, bytes: Vec<u8>) -> Result<()> {
    s3.put_object()
        .bucket(bucket)
        .key(key)
        .content_type("application/json")
        .body(ByteStream::from(bytes))
        .send()
        .await
        .with_context(|| format!("write index artifact to s3://{bucket}/{key}"))?;
    Ok(())
}

async fn copy_index(
    s3: &S3Client,
    bucket: &str,
    source_key: &str,
    destination_key: &str,
) -> Result<()> {
    s3.copy_object()
        .bucket(bucket)
        .key(destination_key)
        .copy_source(format!("{bucket}/{source_key}"))
        .content_type("application/json")
        .metadata_directive(aws_sdk_s3::types::MetadataDirective::Replace)
        .send()
        .await
        .with_context(|| {
            format!("copy index artifact from s3://{bucket}/{source_key} to s3://{bucket}/{destination_key}")
        })?;
    Ok(())
}

fn versioned_index_key() -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("indexes/versions/{now_ms}/index.json")
}
