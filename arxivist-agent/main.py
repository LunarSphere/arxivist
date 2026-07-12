import json
import os
import secrets
from typing import Any

from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
from langchain.messages import AIMessage, HumanMessage, ToolMessage
from pydantic import BaseModel, Field

from graph import agent_build
from tools import reset_request_context, search_places, set_request_context


app = FastAPI(title="Arxivist Agent API")
app.add_middleware(
    CORSMiddleware,
    allow_origins=[
        origin.strip()
        for origin in os.getenv("ARXIVIST_AGENT_CORS_ORIGINS", "*").split(",")
        if origin.strip()
    ],
    allow_methods=["GET", "POST", "OPTIONS"],
    allow_headers=["content-type"],
)
agent = agent_build()


@app.middleware("http")
async def require_proxy_secret(request, call_next):
    """Require the Vercel proxy secret in production, but not for ECS health checks."""
    expected = os.getenv("ARXIVIST_PROXY_SHARED_SECRET")
    if request.url.path != "/health" and expected:
        provided = request.headers.get("x-arxivist-proxy-secret", "")
        if not secrets.compare_digest(provided, expected):
            return JSONResponse(status_code=401, content={"detail": "unauthorized"})
    return await call_next(request)


class UserContext(BaseModel):
    timezone: str | None = None
    locale: str | None = None
    local_time: str | None = None
    approx_location: str | None = Field(
        default=None,
        description="Caller-provided approximate location, such as city/region or ZIP code.",
    )
    latitude: float | None = Field(default=None, ge=-90, le=90)
    longitude: float | None = Field(default=None, ge=-180, le=180)


class NearbyPlace(BaseModel):
    id: str
    name: str
    category: str
    match_category: str = "general"
    latitude: float
    longitude: float
    distance_m: int | None = None
    address: str | None = None
    website: str | None = None
    opening_hours: str | None = None


class AgentSearchRequest(BaseModel):
    query: str = Field(min_length=1)
    top_k: int = Field(default=10, ge=1, le=20)
    context: UserContext | None = None
    nearby_places: list[NearbyPlace] = Field(default_factory=list, max_length=50)


class Source(BaseModel):
    title: str | None = None
    url: str
    snippet: str | None = None


class ToolCallSummary(BaseModel):
    name: str
    args: dict[str, Any]


class AgentSearchResponse(BaseModel):
    query: str
    answer: str
    sources: list[Source]
    tool_calls: list[ToolCallSummary]
    tool_call_count: int
    llm_calls: int


class PlaceSearchRequest(BaseModel):
    query: str = Field(min_length=1, max_length=120)
    latitude: float | None = Field(default=None, ge=-90, le=90)
    longitude: float | None = Field(default=None, ge=-180, le=180)
    location: str | None = Field(default=None, max_length=160)
    radius_m: int = Field(default=5_000, ge=500, le=10_000)
    limit: int = Field(default=50, ge=1, le=50)


class PlaceCenter(BaseModel):
    latitude: float
    longitude: float
    label: str | None = None


class PlaceSearchResponse(BaseModel):
    query: str
    center: PlaceCenter
    radius_m: int
    places: list[NearbyPlace]
    source: str = "overpass"
    attribution: str


@app.get("/health")
def health():
    return {"status": "ok", "service": "agent"}


@app.get("/agent/health")
def agent_health():
    """Compatibility route for the Vercel /api/agent/health proxy path."""
    return health()


@app.post("/places/search", response_model=PlaceSearchResponse)
def places_search(request: PlaceSearchRequest):
    if (request.latitude is None) != (request.longitude is None) and not request.location:
        raise HTTPException(
            status_code=422,
            detail="latitude and longitude must be provided together",
        )
    try:
        return search_places(
            request.query,
            latitude=request.latitude,
            longitude=request.longitude,
            location=request.location,
            radius_m=request.radius_m,
            limit=request.limit,
        )
    except ValueError as error:
        raise HTTPException(status_code=422, detail=str(error)) from error
    except Exception as error:
        raise HTTPException(status_code=502, detail=f"OpenStreetMap search failed: {error}") from error


@app.post("/agent/search", response_model=AgentSearchResponse)
def agent_search(request: AgentSearchRequest):
    context = request.context.model_dump(exclude_none=True) if request.context else {}
    context["top_k"] = request.top_k
    local_context = ""
    if request.nearby_places or context.get("latitude") is not None:
        local_context = (
            "\nLocal map context from OpenStreetMap is included below. Use these places "
            "for local recommendations and do not invent additional real businesses. "
            "If the list is empty, say that local map data was unavailable.\n"
            f"User coordinates: {json.dumps({'latitude': context.get('latitude'), 'longitude': context.get('longitude')})}\n"
            f"Nearby places: {json.dumps([place.model_dump(exclude_none=True) for place in request.nearby_places], ensure_ascii=False)}"
        )
    token = set_request_context(context)
    try:
        result = agent.invoke(
            {
                "messages": [
                    HumanMessage(
                        content=(
                            f"Query: {request.query}\n"
                            f"Requested search result count: {request.top_k}"
                            f"{local_context}"
                        )
                    )
                ],
                "tool_call_count": 0,
                "llm_calls": 0,
            },
            {"recursion_limit": 30},
        )
    finally:
        reset_request_context(token)

    messages = result["messages"]
    return AgentSearchResponse(
        query=request.query,
        answer=_last_answer(messages),
        sources=_sources_from_messages(messages),
        tool_calls=_tool_calls_from_messages(messages),
        tool_call_count=result.get("tool_call_count", 0),
        llm_calls=result.get("llm_calls", 0),
    )


def _last_answer(messages: list[Any]) -> str:
    for message in reversed(messages):
        if isinstance(message, AIMessage) and not getattr(message, "tool_calls", None):
            return _content_to_text(message.content)
    return ""


def _tool_calls_from_messages(messages: list[Any]) -> list[ToolCallSummary]:
    calls: list[ToolCallSummary] = []
    for message in messages:
        for tool_call in getattr(message, "tool_calls", None) or []:
            calls.append(
                ToolCallSummary(
                    name=tool_call["name"],
                    args=dict(tool_call.get("args") or {}),
                )
            )
    return calls


def _sources_from_messages(messages: list[Any]) -> list[Source]:
    by_url: dict[str, Source] = {}
    for message in messages:
        if not isinstance(message, ToolMessage):
            continue
        payload = _decode_json_object(message.content)
        if not payload or not payload.get("ok"):
            continue

        if payload.get("tool") == "search":
            for result in payload.get("results", []):
                url = result.get("url")
                if url and url not in by_url:
                    by_url[url] = Source(
                        title=result.get("title"),
                        url=url,
                        snippet=result.get("snippet"),
                    )
            continue

        url = payload.get("url")
        if url and url not in by_url:
            by_url[url] = Source(
                title=payload.get("title"),
                url=url,
                snippet=payload.get("text", "")[:240] or None,
            )

    return list(by_url.values())


def _decode_json_object(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, str):
        return None
    try:
        decoded = __import__("json").loads(value)
    except ValueError:
        return None
    return decoded if isinstance(decoded, dict) else None


def _content_to_text(content: Any) -> str:
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        parts = []
        for item in content:
            if isinstance(item, dict) and item.get("type") == "text":
                parts.append(item.get("text", ""))
            else:
                parts.append(str(item))
        return "\n".join(part for part in parts if part)
    return str(content)


def main():
    print("Run with: uvicorn main:app --reload")


if __name__ == "__main__":
    main()
