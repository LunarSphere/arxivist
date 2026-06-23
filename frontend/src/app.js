const config = window.ARXIVIST_CONFIG ?? {};
const apiBaseUrl = (config.apiBaseUrl ?? "").replace(/\/$/, "");

const form = document.querySelector("#search-form");
const queryInput = document.querySelector("#query");
const message = document.querySelector("#message");
const indexStatus = document.querySelector("#index-status");
const results = document.querySelector("#results");
const submitButton = form.querySelector("button");

function setMessage(text, tone = "default") {
  message.textContent = text;
  message.dataset.tone = tone;
}

function apiUrl(path) {
  if (!apiBaseUrl) {
    throw new Error("Missing ARXIVIST_API_BASE_URL");
  }
  return `${apiBaseUrl}${path}`;
}

async function loadHealth() {
  try {
    const response = await fetch(apiUrl("/health"));
    if (!response.ok) {
      throw new Error(`Health check failed with ${response.status}`);
    }
    const health = await response.json();
    indexStatus.textContent = `${health.documents.toLocaleString()} pages indexed across ${health.terms.toLocaleString()} terms`;
  } catch (error) {
    indexStatus.textContent = apiBaseUrl ? "Index status unavailable" : "Set ARXIVIST_API_BASE_URL in Vercel";
  }
}

function renderResults(items) {
  results.replaceChildren(
    ...items.map((item) => {
      const row = document.createElement("li");
      row.className = "result";

      const link = document.createElement("a");
      link.href = item.url;
      link.textContent = item.title || item.url;
      link.target = "_blank";
      link.rel = "noreferrer";

      const url = document.createElement("p");
      url.className = "result-url";
      url.textContent = item.url;

      const snippet = document.createElement("p");
      snippet.className = "result-snippet";
      snippet.textContent = item.snippet || "";

      const meta = document.createElement("p");
      meta.className = "result-meta";
      meta.textContent = `Score ${item.score.toFixed(3)} · PageRank ${item.page_rank.toFixed(3)}`;

      row.append(link, url, snippet, meta);
      return row;
    })
  );
}

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const query = queryInput.value.trim();
  if (!query) {
    return;
  }

  submitButton.disabled = true;
  setMessage("Searching...");
  results.replaceChildren();

  try {
    const response = await fetch(apiUrl("/search"), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ query, top_k: 10, mode: "traditional" })
    });

    if (!response.ok) {
      throw new Error(`Search failed with ${response.status}`);
    }

    const payload = await response.json();
    renderResults(payload.results);
    setMessage(payload.results.length === 0 ? "No results found." : `${payload.results.length} results`);
  } catch (error) {
    setMessage(error.message, "error");
  } finally {
    submitButton.disabled = false;
  }
});

loadHealth();
