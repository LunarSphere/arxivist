# Arxivist Crawler

This is the production-minded crawler service for the next version of Arxivist. It leaves `Legacy/` intact and uses `spider` as the crawl engine.

## Shape

- One long-running Rust HTTP service.
- One ECS Fargate task for the crawler demo.
- `spider` owns the in-process crawl frontier.
- Raw HTML is stored in S3.
- Crawl and page metadata are stored in DynamoDB.
- Redis, SQS, browser rendering, proxy rotation, and Spider Cloud are not part of v1.

## API

```http
GET /health
```

```http
POST /crawl
Content-Type: application/json

{
  "seed_urls": ["https://example.com"],
  "max_pages": 5000,
  "depth_limit": 6
}
```

```http
GET /crawl/{crawl_id}
```

The service accepts one active crawl at a time. A second `POST /crawl` returns `409 Conflict` while a crawl is running.

## Configuration

Environment variables:

- `BIND_ADDR`: HTTP bind address. Defaults to `0.0.0.0:8080`.
- `AWS_REGION`: AWS region used by the default AWS SDK provider chain.
- `CRAWLER_S3_BUCKET`: S3 bucket for raw HTML.
- `CRAWLER_DDB_CRAWLS_TABLE`: DynamoDB table for crawl records.
- `CRAWLER_DDB_PAGES_TABLE`: DynamoDB table for page records.
- `CRAWLER_USER_AGENT`: optional crawler user agent.
- `CRAWLER_REQUEST_TIMEOUT_SECS`: optional request timeout. Defaults to `20`.
- `CRAWLER_CRAWL_TIMEOUT_SECS`: optional whole-crawl timeout. Defaults to `900`.
- `CRAWLER_DELAY_MS`: optional spider delay between requests. Defaults to `250`.

Expected DynamoDB keys:

- Crawl table: partition key `crawl_id` as a string.
- Page table: partition key `crawl_id` as a string and sort key `url_hash` as a string.

S3 objects are written under:

```text
{crawl_id}/raw/{url_hash}.html
```

## Local Commands

```bash
cd crawler
cargo test
cargo run
```

Running the service without AWS credentials is useful for compile checks, but `POST /crawl` requires configured AWS resources.
