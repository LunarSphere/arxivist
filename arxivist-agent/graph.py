import operator
import json
import os
from functools import lru_cache
from typing import Literal

from langchain.chat_models import init_chat_model
from langchain.messages import AnyMessage, SystemMessage, ToolMessage
from langgraph.graph import END, START, StateGraph
from typing_extensions import Annotated, TypedDict

from tools import (
    fetch_live_page,
    fetch_local_info,
    fetch_stored_page,
    fetch_user_info,
    search,
)

MAX_TOOL_CALLS = 10
tools = [search, fetch_stored_page, fetch_live_page, fetch_user_info, fetch_local_info]
tools_by_name = {tool.name: tool for tool in tools}


@lru_cache(maxsize=1)
def _model():
    model_name = os.getenv("ARXIVIST_AGENT_MODEL", "openai:gpt-5.4-mini")
    return init_chat_model(model_name, temperature=0)


@lru_cache(maxsize=1)
def _model_with_tools():
    return _model().bind_tools(tools)


class MessagesState(TypedDict, total=False):
    messages: Annotated[list[AnyMessage], operator.add]
    tool_call_count: int
    llm_calls: int


SYSTEM_PROMPT = """
You are Arxivist, a research assistant for answering user queries from the
Arxivist search corpus and supporting web/context tools.

Use tools when they will improve factual accuracy. Prefer the `search` tool for
corpus discovery, then inspect stored or live pages only when snippets are not
enough. You may call tools at most 10 times total.

When you have enough information, write a concise answer that summarizes what
you found. Include a "Sources" section with the URLs that directly supported the
answer. Do not invent sources. If a tool fails or returns no useful results,
state the limitation plainly.
"""


FINAL_PROMPT = """
Write the final answer now using the tool observations already available.
Summarize what was found and include a "Sources" section with URLs used. Do not
request more tools.
"""


def llm_call(state: MessagesState) -> dict:
    """Ask the model whether to use a tool or produce the final answer."""
    response = _model_with_tools().invoke([SystemMessage(content=SYSTEM_PROMPT)] + state["messages"])
    return {
        "messages": [response],
        "llm_calls": state.get("llm_calls", 0) + 1,
    }


def tool_node(state: MessagesState) -> dict:
    """Run tool calls requested by the model, respecting the global call budget."""
    last_message = state["messages"][-1]
    current_count = state.get("tool_call_count", 0)
    remaining = MAX_TOOL_CALLS - current_count
    results: list[ToolMessage] = []
    executed = 0

    for tool_call in last_message.tool_calls:
        if remaining <= 0:
            results.append(
                ToolMessage(
                    content='{"ok": false, "error": "tool call budget reached"}',
                    tool_call_id=tool_call["id"],
                )
            )
            continue

        tool = tools_by_name.get(tool_call["name"])
        if tool is None:
            observation = json.dumps(
                {"ok": False, "error": f"unknown tool: {tool_call['name']}"}
            )
        else:
            try:
                observation = tool.invoke(tool_call["args"])
            except Exception as error:
                observation = json.dumps({"ok": False, "error": str(error)})

        results.append(ToolMessage(content=str(observation), tool_call_id=tool_call["id"]))
        remaining -= 1
        executed += 1

    return {
        "messages": results,
        "tool_call_count": current_count + executed,
    }


def final_answer(state: MessagesState) -> dict:
    """Force a final answer when the tool budget has been used."""
    response = _model().invoke([SystemMessage(content=FINAL_PROMPT)] + state["messages"])
    return {
        "messages": [response],
        "llm_calls": state.get("llm_calls", 0) + 1,
    }


def should_continue(state: MessagesState) -> Literal["tool_node", "__end__"]:
    """Continue only when the model requested tools and budget remains."""
    last_message = state["messages"][-1]
    if getattr(last_message, "tool_calls", None) and state.get("tool_call_count", 0) < MAX_TOOL_CALLS:
        return "tool_node"
    return END


def after_tools(state: MessagesState) -> Literal["llm_call", "final_answer"]:
    if state.get("tool_call_count", 0) >= MAX_TOOL_CALLS:
        return "final_answer"
    return "llm_call"


def agent_build():
    agent_builder = StateGraph(MessagesState)
    agent_builder.add_node("llm_call", llm_call)
    agent_builder.add_node("tool_node", tool_node)
    agent_builder.add_node("final_answer", final_answer)

    agent_builder.add_edge(START, "llm_call")
    agent_builder.add_conditional_edges("llm_call", should_continue)
    agent_builder.add_conditional_edges("tool_node", after_tools)
    agent_builder.add_edge("final_answer", END)

    return agent_builder.compile()


if __name__ == "__main__":
    print("This is graph.py")
