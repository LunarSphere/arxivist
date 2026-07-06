# Architecture

## Component Boundaries

- `arxivist-core` owns shared document types, tokenization, and scoring helpers.
- `arxivist-crawler` fetches pages, extracts links/text, and writes crawl records.
- `arxivist-indexer` builds a versioned search index from crawl records.
- `arxivist-search-api` loads an index artifact and serves search requests.

These boundaries keep the system runnable from local files and local HTTP services.

## Data Flow

1. Seeds enter the crawler as URLs.
2. The crawler stores page metadata plus pointers, with bulky page content in local content files.
3. The indexer reads crawled pages, normalizes terms, builds postings, computes PageRank, and writes one index artifact.
4. The API loads the artifact into memory and ranks requests with BM25, TF-IDF, and PageRank.

## Local Runtime

- Crawl metadata is stored in `pages.jsonl`.
- Raw page snapshots are stored under `content/`.
- Extracted text payloads are stored under `extracted/`.
- Search indexes are stored under `data/dev/index` as either a sharded directory or a legacy JSON file.
- The frontend talks to the local Rust API by default and can use the Vercel proxy when deployed.
