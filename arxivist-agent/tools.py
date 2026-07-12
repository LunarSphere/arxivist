import contextvars
import html.parser
import json
import math
import os
import re
import threading
import time
from collections import OrderedDict
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

_NOMINATIM_MIN_INTERVAL_SECONDS = 1.0
_NOMINATIM_CACHE_SIZE = 128
_nominatim_lock = threading.Lock()
_nominatim_last_request = 0.0
_nominatim_cache: OrderedDict[str, list[dict[str, Any]]] = OrderedDict()
_OVERPASS_MIN_INTERVAL_SECONDS = 1.0
_overpass_lock = threading.Lock()
_overpass_last_request = 0.0
_PLACE_CACHE_TTL_SECONDS = 300.0
_PLACE_EMPTY_CACHE_TTL_SECONDS = 60.0
_PLACE_CACHE_SIZE = 128
_place_cache_lock = threading.Lock()
_place_search_lock = threading.Lock()
_place_cache: OrderedDict[str, tuple[float, dict[str, Any]]] = OrderedDict()


def _osm_headers() -> dict[str, str]:
    """Identify the application as required by the public OSM services."""
    headers = {
        "user-agent": os.getenv(
            "ARXIVIST_OSM_USER_AGENT", "Arxivist/0.1 (local development)"
        ),
    }
    referer = os.getenv("ARXIVIST_OSM_REFERER")
    if referer:
        headers["referer"] = referer
    return headers


def _nominatim_location(location: str, limit: int = 1) -> list[dict[str, Any]]:
    """Geocode one manual location while respecting Nominatim's public limit."""
    global _nominatim_last_request
    key = f"{limit}:" + " ".join(location.lower().split())
    with _nominatim_lock:
        if key in _nominatim_cache:
            value = _nominatim_cache.pop(key)
            _nominatim_cache[key] = value
            return value or []

        wait_seconds = _NOMINATIM_MIN_INTERVAL_SECONDS - (
            time.monotonic() - _nominatim_last_request
        )
        if wait_seconds > 0:
            time.sleep(wait_seconds)

        response = httpx.get(
            "https://nominatim.openstreetmap.org/search",
            params={"q": location, "format": "jsonv2", "limit": limit, "addressdetails": 1},
            headers=_osm_headers(),
            timeout=10.0,
        )
        _nominatim_last_request = time.monotonic()
        response.raise_for_status()
        matches = response.json()
        value = matches
        _nominatim_cache[key] = value
        while len(_nominatim_cache) > _NOMINATIM_CACHE_SIZE:
            _nominatim_cache.popitem(last=False)
        return value


def _normalized_local_query(query: str) -> str:
    return re.sub(
        r"\bnear\s+me\b|\bnearby\b|\baround\s+here\b|\bin\s+my\s+area\b",
        "",
        query.lower(),
    ).strip()


def _place_filter_groups(query: str) -> tuple[str, list[str]]:
    """Map everyday language to bounded groups of common OSM POI tags."""
    normalized = _normalized_local_query(query)

    if any(term in normalized for term in (
        "food", "burger", "pizza", "restaurant", "cafe", "café", "coffee",
        "bakery", "breakfast", "lunch", "dinner", "asian", "mexican", "sushi",
        "bar", "pub", "brewery", "ice cream", "grocery", "supermarket",
    )):
        return "food", [
            '[amenity~"restaurant|fast_food|cafe|food_court|ice_cream|pub|bar|biergarten"]',
            '[shop~"bakery|convenience|supermarket|grocery"]',
        ]

    if any(term in normalized for term in (
        "shop", "shopping", "store", "stores", "mall", "clothes", "clothing",
        "shoes", "electronics", "furniture", "hardware", "books", "bookstore",
        "gift", "florist", "flowers", "cosmetics", "beauty products", "pet store",
        "sporting goods", "market",
    )):
        return "shopping", [
            '[shop~"mall|department_store|clothes|shoes|electronics|furniture|hardware|books|gift|florist|cosmetics|pet|sports|marketplace"]',
            '[amenity="marketplace"]',
        ]

    if any(term in normalized for term in (
        "car dealership", "car dealer", "cars", "automotive", "auto", "mechanic",
        "car repair", "auto repair", "auto parts", "car parts", "tire", "tyre",
        "gas station", "fuel", "car wash", "car rental", "motorcycle",
    )):
        return "automotive", [
            '[shop~"car|car_repair|car_parts|tyres|motorcycle|motorcycle_parts|truck"]',
            '[amenity~"fuel|car_wash"]',
            '[shop="rental"]',
        ]

    if any(term in normalized for term in (
        "pharmacy", "drugstore", "hospital", "clinic", "doctor", "dentist",
        "medical", "health", "optician", "vet", "veterinary",
    )):
        return "health", ['[amenity~"pharmacy|hospital|clinic|doctors|dentist|veterinary"]']

    if any(term in normalized for term in (
        "bank", "atm", "post office", "laundromat", "laundry", "dry cleaner",
        "tailor", "hairdresser", "barber", "salon", "phone store", "printing",
    )):
        return "services", [
            '[amenity~"bank|atm|post_office"]',
            '[shop~"laundry|dry_cleaning|tailor|hairdresser|beauty|mobile_phone|copyshop"]',
        ]

    if any(term in normalized for term in (
        "hotel", "motel", "hostel", "campground", "campsite", "parking", "airport",
        "train station", "bus station", "travel agency",
    )):
        return "travel", [
            '[tourism~"hotel|motel|hostel|camp_site|camp_pitch|information"]',
            '[amenity~"parking|bus_station"]',
            '[railway="station"]',
            '[aeroway="aerodrome"]',
        ]

    if any(term in normalized for term in (
        "park", "playground", "gym", "fitness", "sports", "swimming", "pool",
        "bowling", "golf", "cinema", "movie", "theater", "theatre", "museum",
        "attraction", "dog park", "zoo",
    )):
        return "recreation", [
            '[leisure~"park|playground|fitness_centre|sports_centre|swimming_pool|bowling_alley|golf_course|dog_park"]',
            '[amenity~"cinema|theatre"]',
            '[tourism~"museum|attraction|zoo"]',
        ]

    if any(term in normalized for term in (
        "library", "school", "university", "college", "police", "fire station",
        "town hall", "community center", "community centre", "church", "mosque",
        "temple", "synagogue", "toilet",
    )):
        return "civic", [
            '[amenity~"library|school|university|college|police|fire_station|townhall|community_centre|place_of_worship|toilets"]',
        ]

    escaped = re.sub(r"[^\w\s-]", "", normalized).strip()[:80]
    if not escaped:
        return "general", ["[name]"]
    return "general", [f'[name~"{re.escape(escaped)}",i]']


def _overpass_query(query: str, latitude: float, longitude: float, radius_m: int) -> str:
    _, filters = _place_filter_groups(query)
    parts = [
        f"node(around:{radius_m},{latitude},{longitude}){filter_value};"
        for filter_value in filters
    ]
    parts.extend(
        f"way(around:{radius_m},{latitude},{longitude}){filter_value};"
        for filter_value in filters
    )
    parts.extend(
        f"relation(around:{radius_m},{latitude},{longitude}){filter_value};"
        for filter_value in filters
    )
    return "[out:json][timeout:8];(" + "".join(parts) + ");out center tags;"


def _distance_m(latitude: float, longitude: float, center_latitude: float, center_longitude: float) -> int:
    lat_scale = 111_320
    lon_scale = 111_320 * math.cos(math.radians(center_latitude))
    return round(math.hypot(
        (latitude - center_latitude) * lat_scale,
        (longitude - center_longitude) * lon_scale,
    ))


def _place_from_element(
    element: dict[str, Any], match_category: str = "general"
) -> dict[str, Any] | None:
    tags = element.get("tags") or {}
    name = tags.get("name")
    if not name:
        return None
    center = element.get("center") or {}
    latitude = element.get("lat", center.get("lat"))
    longitude = element.get("lon", center.get("lon"))
    if latitude is None or longitude is None:
        return None
    address_parts = [
        " ".join(part for part in (tags.get("addr:housenumber"), tags.get("addr:street")) if part),
        tags.get("addr:city") or tags.get("addr:town") or tags.get("addr:village"),
        tags.get("addr:postcode"),
    ]
    address = ", ".join(part for part in address_parts if part) or None
    category = (
        tags.get("amenity")
        or tags.get("shop")
        or tags.get("leisure")
        or tags.get("tourism")
        or "place"
    )
    return {
        "id": f"{element.get('type')}/{element.get('id')}",
        "name": name,
        "category": category.replace("_", " "),
        "match_category": match_category,
        "latitude": float(latitude),
        "longitude": float(longitude),
        "address": address,
        "website": tags.get("website") or tags.get("contact:website"),
        "opening_hours": tags.get("opening_hours"),
    }


def _place_from_nominatim(match: dict[str, Any]) -> dict[str, Any] | None:
    latitude = match.get("lat")
    longitude = match.get("lon")
    if latitude is None or longitude is None:
        return None
    return {
        "id": f"nominatim/{match.get('place_id', match.get('osm_id', 'unknown'))}",
        "name": match.get("name") or match.get("display_name") or "Unnamed place",
        "category": (match.get("type") or match.get("category") or "place").replace("_", " "),
        "match_category": "fallback",
        "latitude": float(latitude),
        "longitude": float(longitude),
        "address": match.get("display_name"),
        "website": None,
        "opening_hours": None,
    }


def _overpass_endpoints() -> list[str]:
    configured = os.getenv("ARXIVIST_OVERPASS_URLS")
    if configured:
        return [endpoint.strip() for endpoint in configured.split(",") if endpoint.strip()]
    return [
        os.getenv("ARXIVIST_OVERPASS_URL", "https://overpass-api.de/api/interpreter"),
        "https://overpass.kumi.systems/api/interpreter",
    ]


def _overpass_request(query: str) -> dict[str, Any]:
    """Serialize public Overpass requests and fail over when an instance is busy."""
    global _overpass_last_request
    with _overpass_lock:
        for endpoint in _overpass_endpoints():
            wait_seconds = _OVERPASS_MIN_INTERVAL_SECONDS - (
                time.monotonic() - _overpass_last_request
            )
            if wait_seconds > 0:
                time.sleep(wait_seconds)

            for attempt in range(2):
                try:
                    response = httpx.post(
                        endpoint,
                        data=query,
                        headers=_osm_headers(),
                        timeout=8.0,
                    )
                except httpx.TimeoutException:
                    _overpass_last_request = time.monotonic()
                    raise RuntimeError("OpenStreetMap Overpass request timed out")
                except httpx.TransportError:
                    _overpass_last_request = time.monotonic()
                    raise RuntimeError("OpenStreetMap Overpass request failed")
                _overpass_last_request = time.monotonic()
                if response.status_code in {429, 502, 503, 504} and attempt == 0:
                    retry_after = response.headers.get("retry-after")
                    try:
                        delay = max(1.0, min(float(retry_after), 10.0)) if retry_after else 2.0
                    except ValueError:
                        delay = 2.0
                    time.sleep(delay)
                    continue
                if response.status_code in {429, 502, 503, 504}:
                    break
                response.raise_for_status()
                return response.json()

        raise RuntimeError("all configured OpenStreetMap Overpass services are busy or unavailable")


def _nominatim_nearby_places(
    query: str, latitude: float, longitude: float, limit: int, match_category: str
) -> list[dict[str, Any]]:
    """Use geocoding as a bounded fallback when Overpass is unavailable."""
    normalized_query = re.sub(r"\bnear\s+me\b|\bnearby\b", "", query, flags=re.IGNORECASE).strip()
    normalized_lower = normalized_query.lower()
    if match_category == "food":
        fallback_query = "cafe" if any(term in normalized_lower for term in ("coffee", "cafe", "café")) else "restaurant"
    elif match_category == "automotive":
        fallback_query = "car dealership" if any(term in normalized_lower for term in ("dealer", "dealership", "cars")) else "car repair"
    else:
        fallback_query = {
            "shopping": "shop",
            "health": "pharmacy",
            "services": "service",
            "travel": "hotel",
            "recreation": "gym",
            "civic": "library",
        }.get(match_category, normalized_query)
    matches = _nominatim_location(
        f"{fallback_query} near {latitude},{longitude}", limit=min(limit, 10)
    )
    places: list[dict[str, Any]] = []
    for match in matches:
        place = _place_from_nominatim(match)
        if place:
            place["match_category"] = match_category
            places.append(place)
    return places


def _search_places_uncached(
    query: str,
    *,
    latitude: float | None = None,
    longitude: float | None = None,
    location: str | None = None,
    radius_m: int = 5_000,
    limit: int = 50,
) -> dict[str, Any]:
    """Resolve a location and find nearby OSM places without involving the LLM."""
    if latitude is None or longitude is None:
        if not location:
            raise ValueError("coordinates or a manual location are required")
        geocoded_matches = _nominatim_location(location)
        geocoded = geocoded_matches[0] if geocoded_matches else None
        if not geocoded:
            raise ValueError(f"location not found: {location}")
        latitude = float(geocoded["lat"])
        longitude = float(geocoded["lon"])
        center_label = geocoded.get("display_name") or location
    else:
        center_label = location

    radius_m = max(500, min(int(radius_m), 10_000))
    limit = max(1, min(int(limit), 50))
    match_category, _ = _place_filter_groups(query)
    source = "overpass"
    try:
        elements = _overpass_request(_overpass_query(query, latitude, longitude, radius_m)).get(
            "elements", []
        )
        places: list[dict[str, Any]] = []
        seen: set[str] = set()
        for element in elements:
            place = _place_from_element(element, match_category)
            if place and place["id"] not in seen:
                place["distance_m"] = _distance_m(
                    place["latitude"], place["longitude"], latitude, longitude
                )
                seen.add(place["id"])
                places.append(place)
            if len(places) >= limit:
                break
        places.sort(key=lambda place: place["distance_m"])
        if not places:
            source = "nominatim-fallback"
            places = _nominatim_nearby_places(
                query, latitude, longitude, limit, match_category
            )
    except (httpx.HTTPError, RuntimeError):
        source = "nominatim-fallback"
        places = _nominatim_nearby_places(
            query, latitude, longitude, limit, match_category
        )

    for place in places:
        place["distance_m"] = _distance_m(
            place["latitude"], place["longitude"], latitude, longitude
        )
    places.sort(key=lambda place: place["distance_m"])

    return {
        "query": query,
        "center": {
            "latitude": latitude,
            "longitude": longitude,
            "label": center_label,
        },
        "radius_m": radius_m,
        "places": places,
        "source": source,
        "attribution": "© OpenStreetMap contributors",
    }


def _place_cache_key(
    query: str,
    latitude: float | None,
    longitude: float | None,
    location: str | None,
    radius_m: int,
    limit: int,
) -> str:
    rounded_latitude = round(latitude, 4) if latitude is not None else None
    rounded_longitude = round(longitude, 4) if longitude is not None else None
    normalized_location = " ".join((location or "").lower().split())
    return json.dumps(
        [query.strip().lower(), rounded_latitude, rounded_longitude, normalized_location, radius_m, limit],
        separators=(",", ":"),
    )


def search_places(
    query: str,
    *,
    latitude: float | None = None,
    longitude: float | None = None,
    location: str | None = None,
    radius_m: int = 5_000,
    limit: int = 50,
) -> dict[str, Any]:
    """Return cached/coalesced nearby places before contacting public OSM services."""
    cache_key = _place_cache_key(query, latitude, longitude, location, radius_m, limit)
    now = time.monotonic()
    with _place_cache_lock:
        cached = _place_cache.get(cache_key)
        if cached:
            expires_at, value = cached
            if expires_at > now:
                _place_cache.move_to_end(cache_key)
                return value
            _place_cache.pop(cache_key, None)

    # The provider client is already rate-limited. This outer lock also makes
    # identical concurrent browser requests share the result after the first
    # request completes instead of issuing duplicate OSM queries.
    with _place_search_lock:
        with _place_cache_lock:
            cached = _place_cache.get(cache_key)
            if cached and cached[0] > time.monotonic():
                _place_cache.move_to_end(cache_key)
                return cached[1]

        value = _search_places_uncached(
            query,
            latitude=latitude,
            longitude=longitude,
            location=location,
            radius_m=radius_m,
            limit=limit,
        )
        ttl = _PLACE_EMPTY_CACHE_TTL_SECONDS if not value["places"] else _PLACE_CACHE_TTL_SECONDS
        with _place_cache_lock:
            _place_cache[cache_key] = (time.monotonic() + ttl, value)
            _place_cache.move_to_end(cache_key)
            while len(_place_cache) > _PLACE_CACHE_SIZE:
                _place_cache.popitem(last=False)
        return value


def set_request_context(context: dict[str, Any] | None):
    """Keep request-scoped user context available to tools without global mutation."""
    return REQUEST_CONTEXT.set(context or {})


def reset_request_context(token) -> None:
    REQUEST_CONTEXT.reset(token)


def _json(data: dict[str, Any]) -> str:
    return json.dumps(data, ensure_ascii=False)


def _search_api_base_url() -> str:
    return os.getenv("ARXIVIST_SEARCH_API_BASE_URL", "http://127.0.0.1:3000").rstrip("/")


def _search_api_headers() -> dict[str, str]:
    """Forward the internal proxy credential when the search API runs protected."""
    headers = {"content-type": "application/json"}
    secret = os.getenv("ARXIVIST_PROXY_SHARED_SECRET")
    if secret:
        headers["x-arxivist-proxy-secret"] = secret
    return headers


def _local_data_dir() -> Path:
    return Path(os.getenv("ARXIVIST_LOCAL_DATA_DIR", "data/dev"))


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
            headers=_search_api_headers(),
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
    Reads local pages.jsonl metadata and content files.
    """
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
            "latitude": context.get("latitude"),
            "longitude": context.get("longitude"),
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

    try:
        geocoded = _nominatim_location(f"{query} near {location}", limit=5)
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

    places = geocoded or []

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
