# Arxivist

Arxivist is the production rewrite of the legacy Rust search-engine learning project.

The current implementation starts with a local development pipeline:

1. Crawl pages into local metadata/content files.
2. Build a BM25/TF-IDF/PageRank index artifact.
3. Serve traditional search through a Rust HTTP API.

`Legacy/` is read-only reference material and is intentionally not part of the new workspace.

## Local Pipeline

```bash
cargo run -p arxivist-crawler -- --seed https://books.toscrape.com/ --max-pages 25
cargo run -p arxivist-indexer
cargo run -p arxivist-search-api
```

Then query:

```bash
curl -s http://127.0.0.1:3000/health
curl -s -X POST http://127.0.0.1:3000/search \
  -H 'content-type: application/json' \
  -d '{"query":"book mystery","top_k":5,"mode":"traditional"}'
```

## Production Direction

The local file stores will be replaced with AWS adapters:

- ECS Fargate for crawler, indexer, and API containers.
- SQS for durable crawl frontier jobs.
- DynamoDB for crawl metadata and active index pointers.
- S3 for raw content snapshots and versioned index artifacts.
- Athena for offline inspection of crawl data in S3.
