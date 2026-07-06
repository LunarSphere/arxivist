# Architecture

## Component Boundaries

- `arxivist-core` owns shared document types, tokenization, and scoring helpers.
- `arxivist-crawler` fetches pages, extracts links/text, and writes crawl records.
- `arxivist-indexer` builds a versioned search index from crawl records.
- `arxivist-search-api` loads an index artifact and serves search requests.

These boundaries mirror the planned AWS deployment while staying runnable on a laptop.

## Data Flow

1. Seeds enter the crawler as URLs.
2. The crawler stores page metadata plus pointers, with bulky page content in S3/local content files.
3. The indexer reads crawled pages, normalizes terms, builds postings, computes PageRank, and writes one index artifact.
4. The API loads the artifact into memory and ranks requests with BM25, TF-IDF, and PageRank.

## AWS Mapping

- Local `pages.jsonl` becomes DynamoDB metadata. In AWS, DynamoDB keeps compact crawl facts and S3 keys; it does not store full extracted page text or large link arrays.
- Local `content/` files become S3 page snapshots. AWS also stores extracted indexing payloads under `crawl/extracted/`.
- The in-process crawl queue becomes SQS. -> persistent incase a cralwer crashes + shared queue between multiple crawlers
- Local `index.json` becomes a versioned S3 artifact.
- Local API startup index loading becomes Fargate startup loading from S3.

The AWS indexer reads compact page metadata from DynamoDB, fetches extracted text/link payloads
from S3 for stored pages, and still supports legacy `record_json` items from earlier crawls.

Athena is reserved for offline SQL inspection over S3 data, not interactive search serving.

The application binaries keep local storage as their default runtime and switch to these AWS
adapters only when `--storage aws` or `ARXIVIST_STORAGE_MODE=aws` is set.
