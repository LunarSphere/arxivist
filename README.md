# Arxivist

Arxivist is an agentic search engine for 

The current implementation starts with a local development pipeline:

1. Crawl pages into local metadata/content files.
2. Build a BM25/TF-IDF/PageRank index artifact.
3. Serve traditional search through a Rust HTTP API.
4. Serve agentic search through the arxivist-agent

`Legacy/` is the original version of the search engine that I wrote from scratch.
it is read-only reference material and is intentionally not part of the new workspace.


## Local Pipeline

For a broad local English crawl, run one crawler process with 8 async workers.
`--target-stored-pages` is the goal for saved/indexable pages; `--max-pages` is
the attempted URL safety cap for skipped, blocked, and failed pages.

```bash
cargo run -p arxivist-crawler -- \
  --concurrency 8 \
  --target-stored-pages 64000 \
  --max-pages 256000 \
  --max-depth 6 \
  --delay-ms 700 \
  --output-dir data/dev/crawl \
  --seed https://www.riotgames.com/en \
  --seed https://www.formula1.com/ \
  --seed https://en.wikipedia.org/wiki/Main_Page \
  --seed https://developer.mozilla.org/en-US/ \
  --seed https://doc.rust-lang.org/book/ \
  --seed https://docs.python.org/3/ \
  --seed https://www.espn.com/ \
  --seed https://www.ign.com/ \
  --seed https://news.ycombinator.com/ \
  --seed https://www.britannica.com/
cargo run -p arxivist-indexer -- --output data/dev/index
cargo run -p arxivist-search-api -- --index data/dev/index
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

For Vercel, use the built-in API proxy so the browser can call a hosted search API through the same
HTTPS origin:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=<hosted Rust search API URL>
```

Set the Vercel project root directory to `frontend` so the `api/` directory is deployed with the
static app.

The proxy forwards `GET /api/health` and `POST /api/search` to the Rust API. The browser keeps the
same search request shape as local development:

```json
{ "query": "book mystery", "top_k": 10, "mode": "traditional" }
```

Agentic search is served separately and can be enabled without changing traditional search. For
local development, run the Rust search API and then start the Python agent with your OpenAI key in
the shell environment:

```bash
export OPENAI_API_KEY="<your key>"
export ARXIVIST_SEARCH_API_BASE_URL=http://127.0.0.1:3000
cd arxivist-agent
uv run uvicorn main:app --reload
```

The frontend defaults to the Rust API at `http://127.0.0.1:3000` and the agent API at
`http://127.0.0.1:8000`. For Vercel, keep browser calls same-origin and set:

```text
ARXIVIST_API_BASE_URL=/api
ARXIVIST_AGENT_API_BASE_URL=/api
ARXIVIST_UPSTREAM_API_BASE_URL=<hosted Rust search API URL>
ARXIVIST_UPSTREAM_AGENT_API_BASE_URL=<hosted agent API URL>
```

Do not commit the OpenAI API key or add it to frontend/Vercel public variables. The agent reads it
from its server-side process environment.
