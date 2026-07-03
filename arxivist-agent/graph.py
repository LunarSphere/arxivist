import operator

from langchain.chat_models import init_chat_model
from langchain.messages import AnyMessage, SystemMessage, ToolMessage
from typing_extensions import Annotated, TypedDict

from tools import fetch_live_page, fetch_stored_page, get_user_info

model = init_chat_model("openai:gpt-5.4-mini", temperature=0)

# give the LLMS access to the tools
tools = [fetch_stored_page, fetch_live_page, get_user_info]
tools_by_name = {tool.name: tool for tool in tools}
model_with_tools = model.bind_tools(tools)


# define message state
# I dont beleive we need to operator stuff but i dont know what its for so leave for now.
class MessagesState(TypedDict):
    messages: Annotated[list[AnyMessage], operator.add]
    llm_calls: int


#
def llm_call(state: MessagesState):
    """store an agents system prompt"""

    return {
        "messages": [
            model_with_tools.invoke(
                [
                    SystemMessage(
                        content="""
                        You are a Arxivist an agent tasked with answering the following query:

                        Query: {query}
                        Image Context: {image_context}

                        Your goal is to reason about the query and decide on the best course of action to answer it accurately.

                        Previous reasoning steps and observations: {history}

                        Available tools: {tools}

                        Instructions:
                        1. Analyze the query, previous reasoning steps, and observations.
                        2. Decide on the next action: use a tool or provide a final answer.
                        3. Respond in the following JSON format:

                        If you need to use a tool:
                        {{
                            "thought": "Your detailed reasoning about what to do next",
                            "action": {{
                                "name": "Tool name (wikipedia, google, or none) Example: GOOGLE, WIKIPEDIA, MULTIPLE_CAT_FACTS, etc. Make sure to ignore NAME.",
                                "reason": "Explanation of why you chose this tool",
                                "input": "Specific input for the tool, if different from the original query"
                            }}
                        }}

                        If you have enough information to answer the query:
                        {{
                            "thought": "Your final reasoning process",
                            "answer": "Your comprehensive answer to the query"
                        }}
                        """
                    )
                ]
                + state["messages"]
            )
        ],
        "llm_calls": state.get("llm_calls", 0) + 1,
    }


# Define tool node


def tool_node(state: MessagesState):
    """Performs the tool call"""

    result = []
    for tool_call in state["messages"][-1].tool_calls:  # interesting error
        tool = tools_by_name[tool_call["name"]]
        observation = tool.invoke(tool_call["args"])
        result.append(ToolMessage(content=observation, tool_call_id=tool_call["id"]))
    return {"messages": result}


# deterimine if agent should keep looping,
# wonder if i can control how many times the agent loops
def should_continue(state: MessagesState) -> Literal["tool_node", END]:
    """Decide if we should continue the loop or stop based upon whether the LLM made a tool call"""

    messages = state["messages"]
    last_message = messages[-1]

    # If the LLM makes a tool call, then perform an action
    if last_message.tool_calls:
        return "tool_node"

    # Otherwise, we stop (reply to the user)
    return END


def agent_build():
    # Build workflow
    agent_builder = StateGraph(MessagesState)

    # Add nodes
    agent_builder.add_node("llm_call", llm_call)
    agent_builder.add_node("tool_node", tool_node)

    # Add edges to connect nodes
    agent_builder.add_edge(START, "llm_call")
    agent_builder.add_conditional_edges("llm_call", should_continue, ["tool_node", END])
    agent_builder.add_edge("tool_node", "llm_call")

    return build_agent


if __name__ == "__main__":
    print("This is graph.py")
