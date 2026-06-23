# Architecture

## Component Boundaries

- `arxivist-core` owns shared document types, tokenization, and scoring helpers.
- `arxivist-crawler` fetches pages, extracts links/text, and writes crawl records.
- `arxivist-indexer` builds a versioned search index from crawl records.
- `arxivist-search-api` loads an index artifact and serves search requests.

These boundaries mirror the planned AWS deployment while staying runnable on a laptop.

## Data Flow

1. Seeds enter the crawler as URLs.
2. The crawler stores page metadata plus raw HTML/text.
3. The indexer reads crawled pages, normalizes terms, builds postings, computes PageRank, and writes one index artifact.
4. The API loads the artifact into memory and ranks requests with BM25, TF-IDF, and PageRank.

## AWS Mapping

- Local `pages.jsonl` becomes DynamoDB metadata.
- Local `content/` files become S3 page snapshots.
- The in-process crawl queue becomes SQS.
- Local `index.json` becomes a versioned S3 artifact.
- Local API startup index loading becomes Fargate startup loading from S3.

Athena is reserved for offline SQL inspection over S3 data, not interactive search serving.
