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

Run the frontend from another shell:

```bash
cd frontend
npm run dev
```

The local frontend defaults to `http://127.0.0.1:3000` for API requests. Set
`ARXIVIST_API_BASE_URL` before `npm run build` only when you need a different API base URL.

## Frontend Deployment

The frontend is a static HTML/CSS/JavaScript app in `frontend/`. It builds a generated `config.js`
file from environment variables:

```bash
cd frontend
npm run build
```

For Vercel, use the built-in API proxy so the browser can call the AWS search API through the same
HTTPS origin:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=<SearchApiUrl from the CDK outputs>
```

Set the Vercel project root directory to `frontend` so the `api/` directory is deployed with the
static app.

The proxy forwards `GET /api/health` and `POST /api/search` to the Rust API. The browser keeps the
same search request shape as local development:

```json
{ "query": "book mystery", "top_k": 10, "mode": "traditional" }
```

## Production Direction

The local file stores now have AWS demo adapters selected with `--storage aws` or
`ARXIVIST_STORAGE_MODE=aws`:

- ECS Fargate for crawler, indexer, and API containers.
- SQS for durable crawl frontier jobs.
- DynamoDB for crawl metadata and crawl URL de-duplication.
- S3 for raw content snapshots and versioned index artifacts.
- Athena for offline inspection of crawl data in S3.

Local mode remains the default for every binary so each component can be tested without cloud
access.
