# Arxivist Agent

This service owns agentic search. It stays separate from the Rust search API so the graph can evolve
component by component.

## Local Run

Start the Rust search API first, then run:

```bash
export OPENAI_API_KEY=""
export ARXIVIST_SEARCH_API_BASE_URL=http://127.0.0.1:3000
export ARXIVIST_OSM_USER_AGENT="Arxivist/0.1 (local development)"
export ARXIVIST_OSM_REFERER="http://127.0.0.1:5173/"
uv run uvicorn main:app --reload
```

The agent's `/places/search` endpoint uses OpenStreetMap Nominatim for manual
location lookup and Overpass for nearby place data. Keep requests below one
Nominatim request per second, use an identifying `ARXIVIST_OSM_USER_AGENT`, and
keep the OpenStreetMap attribution visible in clients.
For a hosted deployment, replace the local-development identity with a stable
contact email or project URL.

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
