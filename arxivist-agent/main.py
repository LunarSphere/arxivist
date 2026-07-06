import base64
import os
from typing import Any

from fastapi import FastAPI
from langchain.messages import AIMessage, HumanMessage, ToolMessage
from pydantic import BaseModel, Field

from graph import agent_build
from tools import reset_request_context, set_request_context


app = FastAPI(title="Arxivist Agent API")
agent = agent_build()


class UserContext(BaseModel):
    timezone: str | None = None
    locale: str | None = None
    local_time: str | None = None
    approx_location: str | None = Field(
        default=None,
        description="Caller-provided approximate location, such as city/region or ZIP code.",
    )


class AgentSearchRequest(BaseModel):
    query: str = Field(min_length=1)
    top_k: int = Field(default=10, ge=1, le=20)
    context: UserContext | None = None


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


@app.get("/health")
def health():
    return {"status": "ok", "service": "agent"}


@app.post("/agent/search", response_model=AgentSearchResponse)
def agent_search(request: AgentSearchRequest):
    context = request.context.model_dump(exclude_none=True) if request.context else {}
    context["top_k"] = request.top_k
    token = set_request_context(context)
    try:
        result = agent.invoke(
            {
                "messages": [
                    HumanMessage(
                        content=(
                            f"Query: {request.query}\n"
                            f"Requested search result count: {request.top_k}"
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


def lambda_handler(event: dict[str, Any], context: Any) -> dict[str, Any]:
    """Small API Gateway adapter for the same local FastAPI route contract."""
    method = (
        event.get("requestContext", {})
        .get("http", {})
        .get("method", event.get("httpMethod", ""))
    )
    path = (
        event.get("rawPath")
        or event.get("path")
        or event.get("requestContext", {}).get("http", {}).get("path", "")
    )

    if method == "GET" and path.endswith("/agent/health"):
        return _lambda_json(200, health())

    if method == "POST" and path.endswith("/agent/search"):
        try:
            body = _lambda_body(event)
            request = AgentSearchRequest.model_validate_json(body)
            response = agent_search(request)
        except Exception as error:
            return _lambda_json(400, {"error": str(error)})

        return _lambda_json(200, response.model_dump())

    return _lambda_json(404, {"error": "Not found"})


def _lambda_body(event: dict[str, Any]) -> str:
    body = event.get("body") or "{}"
    if event.get("isBase64Encoded"):
        return base64.b64decode(body).decode("utf-8")
    return body


def _lambda_json(status_code: int, body: dict[str, Any]) -> dict[str, Any]:
    return {
        "statusCode": status_code,
        "headers": {
            "content-type": "application/json",
            "access-control-allow-origin": "*",
            "access-control-allow-methods": "GET,POST,OPTIONS",
            "access-control-allow-headers": "content-type",
        },
        "body": json.dumps(body),
    }


def main():
    print("Run with: uvicorn main:app --reload")


if __name__ == "__main__":
    main()
