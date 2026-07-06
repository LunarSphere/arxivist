# Arxivist Agent

This service owns agentic search. It stays separate from the Rust search API so the graph can evolve
component by component.

## Local Run

Start the Rust search API first, then run:

```bash
export OPENAI_API_KEY="<your key>"
export ARXIVIST_SEARCH_API_BASE_URL=http://127.0.0.1:3000
uv run uvicorn main:app --reload
```

Test it:

```bash
curl http://127.0.0.1:8000/health
curl -s -X POST http://127.0.0.1:8000/agent/search \
  -H 'content-type: application/json' \
  -d '{"query":"transformer retrieval","top_k":5}'
```

## Configuration

The agent reads `OPENAI_API_KEY` from the local process environment. Keep this key server-side; do
not expose it through frontend or Vercel public variables.
