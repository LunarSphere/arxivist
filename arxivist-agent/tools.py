import contextvars
import hashlib
import html.parser
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

import httpx
from langchain.tools import tool

REQUEST_CONTEXT: contextvars.ContextVar[dict[str, Any]] = contextvars.ContextVar(
    "REQUEST_CONTEXT",
    default={},
)


def set_request_context(context: dict[str, Any] | None):
    """Keep request-scoped user context available to tools without global mutation."""
    return REQUEST_CONTEXT.set(context or {})


def reset_request_context(token) -> None:
    REQUEST_CONTEXT.reset(token)


def _json(data: dict[str, Any]) -> str:
    return json.dumps(data, ensure_ascii=False)


def _search_api_base_url() -> str:
    return os.getenv("ARXIVIST_SEARCH_API_BASE_URL", "http://127.0.0.1:3000").rstrip("/")


def _storage_mode() -> str:
    return os.getenv("ARXIVIST_STORAGE_MODE", "local").lower()


def _local_data_dir() -> Path:
    return Path(os.getenv("ARXIVIST_LOCAL_DATA_DIR", "data/dev"))


def _url_hash(url: str) -> str:
    return hashlib.sha256(url.encode("utf-8")).hexdigest()


@tool
def search(query: str, top_k: int = 0) -> str:
    """
    Search the Arxivist index for pages relevant to a query.
    Returns ranked results with title, URL, snippet, and score.
    """
    requested_top_k = top_k or REQUEST_CONTEXT.get().get("top_k", 10)
    top_k = max(1, min(int(requested_top_k), 20))
    try:
        response = httpx.post(
            f"{_search_api_base_url()}/search",
            json={"query": query, "top_k": top_k, "mode": "traditional"},
            timeout=10.0,
        )
        response.raise_for_status()
        payload = response.json()
    except Exception as error:
        return _json(
            {
                "ok": False,
                "tool": "search",
                "query": query,
                "error": str(error),
            }
        )

    results = [
        {
            "title": result.get("title") or result.get("url"),
            "url": result.get("url"),
            "snippet": result.get("snippet", ""),
            "score": result.get("score"),
        }
        for result in payload.get("results", [])
    ]
    return _json(
        {
            "ok": True,
            "tool": "search",
            "query": payload.get("query", query),
            "mode": payload.get("mode", "traditional"),
            "results": results,
        }
    )


@tool
def fetch_stored_page(url: str) -> str:
    """
    Fetch a page snapshot from Arxivist storage for a URL.
    Local mode reads pages.jsonl and content files; AWS mode reads DynamoDB and S3.
    """
    if _storage_mode() == "aws":
        return _fetch_stored_page_aws(url)
    return _fetch_stored_page_local(url)


def _fetch_stored_page_local(url: str) -> str:
    data_dir = _local_data_dir()
    pages_path = data_dir / "pages.jsonl"
    if not pages_path.exists():
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": f"local crawl metadata not found at {pages_path}",
            }
        )

    record = None
    try:
        with pages_path.open("r", encoding="utf-8") as pages:
            for line in pages:
                if not line.strip():
                    continue
                candidate = json.loads(line)
                urls = {candidate.get("requested_url"), candidate.get("final_url")}
                if url in urls:
                    record = candidate
                    break
    except Exception as error:
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": str(error),
            }
        )

    if record is None:
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": "no stored crawl record found for URL",
            }
        )

    content_path = record.get("content_path")
    text = record.get("extracted_text") or ""
    if content_path:
        html_path = data_dir / content_path
        if html_path.exists():
            html = html_path.read_text(encoding="utf-8", errors="replace")
            text = _html_to_text(html) or text

    return _json(
        {
            "ok": True,
            "tool": "fetch_stored_page",
            "url": record.get("final_url") or record.get("requested_url") or url,
            "title": record.get("title"),
            "source": "local",
            "text": _truncate(text, 6_000),
        }
    )


def _fetch_stored_page_aws(url: str) -> str:
    try:
        import boto3
        from boto3.dynamodb.conditions import Attr
    except ImportError as error:
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": f"boto3 is required for AWS storage mode: {error}",
            }
        )

    bucket = os.getenv("ARXIVIST_DATA_BUCKET")
    table_name = os.getenv("ARXIVIST_PAGES_TABLE")
    if not bucket or not table_name:
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": "ARXIVIST_DATA_BUCKET and ARXIVIST_PAGES_TABLE are required",
            }
        )

    try:
        dynamodb = boto3.resource("dynamodb")
        table = dynamodb.Table(table_name)
        response = table.get_item(Key={"url_hash": _url_hash(url)})
        item = response.get("Item")
        if item is None:
            scan = table.scan(
                FilterExpression=Attr("final_url").eq(url) | Attr("requested_url").eq(url),
                Limit=1,
            )
            items = scan.get("Items", [])
            item = items[0] if items else None
        if item is None:
            return _json(
                {
                    "ok": False,
                    "tool": "fetch_stored_page",
                    "url": url,
                    "error": "no stored crawl record found for URL",
                }
            )

        s3 = boto3.client("s3")
        record = _aws_record_from_item(item, s3, bucket)
    except Exception as error:
        return _json(
            {
                "ok": False,
                "tool": "fetch_stored_page",
                "url": url,
                "error": str(error),
            }
        )

    return _json(
        {
            "ok": True,
            "tool": "fetch_stored_page",
            "url": record.get("final_url") or record.get("requested_url") or url,
            "title": record.get("title"),
            "source": "aws",
            "text": _truncate(record.get("text", ""), 6_000),
        }
    )


def _aws_record_from_item(item: dict[str, Any], s3: Any, bucket: str) -> dict[str, Any]:
    payload_path = item.get("extracted_payload_path")
    if payload_path:
        payload = _read_s3_json(s3, bucket, payload_path)
        return {
            "requested_url": item.get("requested_url"),
            "final_url": item.get("final_url"),
            "title": item.get("title"),
            "text": payload.get("extracted_text", ""),
        }

    record = json.loads(item.get("record_json", "{}"))
    text = record.get("extracted_text") or ""
    content_path = record.get("content_path") or item.get("content_path")
    if content_path:
        body = s3.get_object(Bucket=bucket, Key=content_path)["Body"].read()
        text = _html_to_text(body.decode("utf-8", errors="replace")) or text

    return {
        "requested_url": record.get("requested_url") or item.get("requested_url"),
        "final_url": record.get("final_url") or item.get("final_url"),
        "title": record.get("title") or item.get("title"),
        "text": text,
    }


def _read_s3_json(s3: Any, bucket: str, key: str) -> dict[str, Any]:
    body = s3.get_object(Bucket=bucket, Key=key)["Body"].read()
    decoded = json.loads(body.decode("utf-8", errors="replace"))
    return decoded if isinstance(decoded, dict) else {}


@tool
def fetch_live_page(url: str) -> str:
    """
    Fetch and extract readable text from a live URL when no stored page is available.
    """
    parsed = urlparse(url)
    if parsed.scheme not in {"http", "https"}:
        return _json(
            {
                "ok": False,
                "tool": "fetch_live_page",
                "url": url,
                "error": "only http and https URLs are supported",
            }
        )

    try:
        response = httpx.get(
            url,
            follow_redirects=True,
            timeout=10.0,
            headers={"user-agent": "ArxivistAgent/0.1"},
        )
        response.raise_for_status()
    except Exception as error:
        return _json(
            {
                "ok": False,
                "tool": "fetch_live_page",
                "url": url,
                "error": str(error),
            }
        )

    return _json(
        {
            "ok": True,
            "tool": "fetch_live_page",
            "url": str(response.url),
            "title": _html_title(response.text),
            "text": _truncate(_html_to_text(response.text), 6_000),
        }
    )


@tool
def fetch_user_info() -> str:
    """
    Return caller-provided user context plus server time.
    Location is only present when the API caller provides it.
    """
    context = REQUEST_CONTEXT.get({})
    return _json(
        {
            "ok": True,
            "tool": "fetch_user_info",
            "server_time": datetime.now(timezone.utc).isoformat(),
            "timezone": context.get("timezone"),
            "locale": context.get("locale"),
            "local_time": context.get("local_time"),
            "approx_location": context.get("approx_location"),
        }
    )


@tool
def fetch_local_info(query: str, location: str | None = None) -> str:
    """
    Find nearby places or local context through OpenStreetMap Nominatim.
    Requires an approximate location from the request or from this tool call.
    """
    context = REQUEST_CONTEXT.get({})
    location = location or context.get("approx_location")
    if not location:
        return _json(
            {
                "ok": False,
                "tool": "fetch_local_info",
                "query": query,
                "error": "approximate location was not provided by the caller",
            }
        )

    user_agent = os.getenv("ARXIVIST_NOMINATIM_USER_AGENT", "ArxivistAgent/0.1")
    try:
        response = httpx.get(
            "https://nominatim.openstreetmap.org/search",
            params={
                "q": f"{query} near {location}",
                "format": "jsonv2",
                "limit": 5,
                "addressdetails": 1,
            },
            headers={"user-agent": user_agent},
            timeout=10.0,
        )
        response.raise_for_status()
        places = response.json()
    except Exception as error:
        return _json(
            {
                "ok": False,
                "tool": "fetch_local_info",
                "query": query,
                "location": location,
                "error": str(error),
            }
        )

    return _json(
        {
            "ok": True,
            "tool": "fetch_local_info",
            "query": query,
            "location": location,
            "results": [
                {
                    "name": place.get("name") or place.get("display_name"),
                    "display_name": place.get("display_name"),
                    "category": place.get("category"),
                    "type": place.get("type"),
                    "lat": place.get("lat"),
                    "lon": place.get("lon"),
                }
                for place in places
            ],
        }
    )


class _TextExtractor(html.parser.HTMLParser):
    def __init__(self) -> None:
        super().__init__()
        self._skip_depth = 0
        self.title = ""
        self._in_title = False
        self._parts: list[str] = []

    def handle_starttag(self, tag: str, attrs) -> None:
        if tag in {"script", "style", "noscript"}:
            self._skip_depth += 1
        if tag == "title":
            self._in_title = True

    def handle_endtag(self, tag: str) -> None:
        if tag in {"script", "style", "noscript"} and self._skip_depth:
            self._skip_depth -= 1
        if tag == "title":
            self._in_title = False

    def handle_data(self, data: str) -> None:
        text = data.strip()
        if not text:
            return
        if self._in_title:
            self.title = f"{self.title} {text}".strip()
        if self._skip_depth == 0:
            self._parts.append(text)

    @property
    def text(self) -> str:
        return " ".join(" ".join(self._parts).split())


def _parse_html(html: str) -> _TextExtractor:
    parser = _TextExtractor()
    parser.feed(html)
    return parser


def _html_to_text(html: str) -> str:
    return _parse_html(html).text


def _html_title(html: str) -> str | None:
    return _parse_html(html).title or None


def _truncate(text: str, limit: int) -> str:
    text = " ".join(text.split())
    if len(text) <= limit:
        return text
    return f"{text[:limit].rstrip()}..."


if __name__ == "__main__":
    print("This is tools.py")
