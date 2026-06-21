# Arxivist Indexer

This is the build-only indexer service for the next version of Arxivist. It consumes the crawler's DynamoDB/S3 output and produces immutable Tantivy index artifacts for a later search service.

## Shape

- One long-running Rust HTTP service.
- One ECS Fargate task for the indexer demo.
- Build-only: it does not serve search queries.
- Reads crawler page metadata from DynamoDB.
- Reads raw HTML from S3.
- Filters to reliable English pages.
- Builds one Tantivy index artifact per `crawl_id`.
- Computes PageRank over indexed pages and stores it in the Tantivy index and manifest.
- Uploads index files and `manifest.json` to S3.
- Tracks index-build status in DynamoDB.

## API

```http
GET /health
```

```http
POST /index
Content-Type: application/json

{
  "crawl_id": "crawl-uuid"
}
```

```http
GET /index/{index_build_id}
```

The service accepts one active index build at a time. A second `POST /index` returns `409 Conflict` while a build is running.

## Configuration

Environment variables:

- `BIND_ADDR`: HTTP bind address. Defaults to `0.0.0.0:8081`.
- `AWS_REGION`: AWS region used by the default AWS SDK provider chain.
- `INDEXER_CRAWLER_PAGES_TABLE`: crawler page metadata table.
- `INDEXER_BUILDS_TABLE`: index-build status table.
- `INDEXER_RAW_HTML_BUCKET`: crawler raw HTML bucket.
- `INDEXER_INDEX_BUCKET`: bucket for Tantivy index artifacts.
- `INDEXER_WORK_DIR`: local build work directory. Defaults to `/tmp/arxivist-indexer`.
- `INDEXER_MIN_TEXT_CHARS`: minimum extracted text length. Defaults to `200`.
- `INDEXER_LANGUAGE_CONFIDENCE`: minimum `whatlang` confidence. Defaults to `0.80`.
- `INDEXER_PAGERANK_DAMPING`: PageRank damping factor. Defaults to `0.85`.
- `INDEXER_PAGERANK_ITERATIONS`: PageRank iterations. Defaults to `20`.

Expected DynamoDB keys:

- Crawler pages table: partition key `crawl_id` and sort key `url_hash`.
- Index builds table: partition key `index_build_id`.

S3 artifacts are written under:

```text
indexes/{crawl_id}/{index_build_id}/tantivy/
indexes/{crawl_id}/{index_build_id}/manifest.json
```

## Local Commands

```bash
cd indexer
cargo test
cargo run
```

Running the service without AWS credentials is useful for compile checks, but `POST /index` requires configured AWS resources.
