"""
Notes for Kevius
this main.py will handel our api calls
GET /health ->
POST /agent/search -> takes a user query | return a search result

Little reminder of our scope.
when a user query is posted past to chain  it'll call the correct tools and after a couple of iterations
the end user will get a search result. ie architecturally we are building a simple api


"""

from typing import Literal

from fastapi import FastAPI
from langchain.messages import HumanMessage

from graph import agent_build

app = FastAPI(title="Arxivist Agent API")

# graph = build_graph()


@app.get("/health")
def health():
    return {"status": "ok", "service": "agent"}


@app.post("/agent/search")
def agent_search(query: str):
    agent = agent_build()
    messages = [HumanMessage(content=query)]
    messages = agent.invoke({"messages": messages})
    for m in messages["messages"]:
        m.pretty_print()
    return None


def main():
    print("Hello from agent-search!")


if __name__ == "__main__":
    main()
