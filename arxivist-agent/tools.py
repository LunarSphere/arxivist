# in this file we will define tools as simple function defitions

from langchain.tools import tool


@tool
def search(query: str):
    """
    In: query a string representing the terms the user is searching for
    Out: json reponse containing ranked search results from the search api
    """
    return None


@tool
def fetch_stored_page(url: str):
    """
    open and read an individual page
    we will fetch content from an s3 bucket
    """
    return None


@tool
def fetch_live_page(url: str):
    """
    fetch a live url primarily for if the page is not in our s3 bucket
    ie fetch stored page failed
    """
    return None


@tool
def get_user_info():
    """
    return a json containing approx location, timezone, language,
    """
    return None


if __name__ == "__main__":
    print("This is tools.py")
