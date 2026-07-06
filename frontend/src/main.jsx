import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const config = window.ARXIVIST_CONFIG ?? {};
const configuredApiBaseUrl = (config.apiBaseUrl ?? "").trim();
const apiBaseUrl = (configuredApiBaseUrl || "http://127.0.0.1:3000").replace(/\/$/, "");
const configuredAgentApiBaseUrl = (config.agentApiBaseUrl ?? "").trim();
const agentApiBaseUrl = (configuredAgentApiBaseUrl || "http://127.0.0.1:8000").replace(/\/$/, "");
const themeStorageKey = "arxivist-theme";
const searchPageSize = 10;
const agentSourceLimit = 5;

const suggestionSeeds = [
  "graph neural networks",
  "transformer retrieval",
  "quantum error correction",
  "large language models",
  "diffusion models",
  "semantic search",
  "pagerank ranking",
  "rust search engine"
];

function apiUrl(path) {
  return `${apiBaseUrl}${path}`;
}

function agentApiUrl(path) {
  return `${agentApiBaseUrl}${path}`;
}

function formatMetric(value) {
  return Number.isFinite(value) ? value.toFixed(3) : "0.000";
}

function useTheme() {
  const [theme, setTheme] = useState(() => {
    if (typeof localStorage === "undefined") {
      return "dark";
    }
    return localStorage.getItem(themeStorageKey) ?? "dark";
  });

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
    localStorage.setItem(themeStorageKey, theme);
  }, [theme]);

  return [theme, setTheme];
}

function getSuggestions(query) {
  const normalized = query.trim().toLowerCase();
  if (!normalized) {
    return [];
  }

  const matches = suggestionSeeds.filter((seed) => seed.includes(normalized));
  const fallbacks = [
    `${normalized} survey`,
    `${normalized} benchmark`,
    `${normalized} implementation`
  ];

  return [...new Set([...matches, ...fallbacks])].slice(0, 5);
}

function IconButton({ children, label, onClick, pressed }) {
  return (
    <button
      className="icon-button"
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      title={label}
      onClick={onClick}
    >
      {children}
    </button>
  );
}

function ThemeIcon({ theme }) {
  if (theme === "dark") {
    return (
      <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
        <path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" />
      </svg>
    );
  }

  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <circle cx="12" cy="12" r="4" />
      <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
    </svg>
  );
}

function ArrowIcon({ direction }) {
  const path = direction === "previous" ? "M15 18l-6-6 6-6" : "M9 6l6 6-6 6";

  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <path d={path} />
    </svg>
  );
}

function SearchBox({ query, setQuery, onSearch, compact = false, loading = false }) {
  const [focused, setFocused] = useState(false);
  const suggestions = useMemo(() => getSuggestions(query), [query]);
  const showSuggestions = focused && suggestions.length > 0;

  function submit(event) {
    event.preventDefault();
    onSearch(query);
    setFocused(false);
  }

  return (
    <form className={`search-box ${compact ? "search-box-compact" : ""}`} onSubmit={submit}>
      <div className="search-input-wrap">
        <span className="search-icon" aria-hidden="true">
          <svg viewBox="0 0 24 24" focusable="false">
            <circle cx="11" cy="11" r="7" />
            <path d="m16.5 16.5 4 4" />
          </svg>
        </span>
        <label className="visually-hidden" htmlFor={compact ? "results-query" : "home-query"}>
          Search query
        </label>
        <input
          id={compact ? "results-query" : "home-query"}
          value={query}
          type="search"
          placeholder="Search papers, methods, and demo crawl pages"
          autoComplete="off"
          onChange={(event) => setQuery(event.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => window.setTimeout(() => setFocused(false), 120)}
        />
        <button className="search-submit" type="submit" disabled={loading || !query.trim()}>
          Search
        </button>
      </div>

      {showSuggestions ? (
        <div className="suggestions" role="listbox" aria-label="Search suggestions">
          {suggestions.map((suggestion) => (
            <button
              key={suggestion}
              type="button"
              role="option"
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => {
                setQuery(suggestion);
                onSearch(suggestion);
              }}
            >
              <span aria-hidden="true">+</span>
              {suggestion}
            </button>
          ))}
        </div>
      ) : null}
    </form>
  );
}

function ModeSwitch({ searchMode, onModeChange }) {
  return (
    <div className="mode-switch" aria-label="Search mode">
      <button
        className={searchMode === "traditional" ? "active" : ""}
        type="button"
        onClick={() => onModeChange("traditional")}
      >
        Search
      </button>
      <button
        className={searchMode === "agent" ? "active" : ""}
        type="button"
        onClick={() => onModeChange("agent")}
      >
        AI
      </button>
    </div>
  );
}

function SearchHome({
  query,
  setQuery,
  onSearch,
  health,
  theme,
  toggleTheme,
  loading,
  searchMode,
  onModeChange
}) {
  return (
    <main className="home-shell">
      <header className="home-actions" aria-label="Page controls">
        <button className="text-button" type="button" onClick={() => onModeChange("agent")}>AI</button>
        <IconButton label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`} pressed={theme === "dark"} onClick={toggleTheme}>
          <ThemeIcon theme={theme} />
        </IconButton>
        <IconButton label="Menu">Menu</IconButton>
      </header>

      <section className="home-panel" aria-labelledby="page-title">
        <h1 id="page-title" className="brand">Arxivist</h1>
        <ModeSwitch searchMode={searchMode} onModeChange={onModeChange} />
        <SearchBox query={query} setQuery={setQuery} onSearch={onSearch} loading={loading} />
        <p className="tagline">Private, focused search across your indexed research corpus.</p>
        <p className="health-line">{health}</p>
      </section>
    </main>
  );
}

function SearchResults({
  query,
  setQuery,
  onSearch,
  results,
  status,
  message,
  health,
  theme,
  toggleTheme,
  loading,
  currentPage,
  totalPages,
  totalResults,
  hasPrevious,
  hasNext,
  searchMode,
  onModeChange,
  agentAssist
}) {
  const showPagination = totalResults > 0 && totalPages > 0 && !message;

  return (
    <main className="results-page">
      <header className="results-top">
        <button className="mini-brand" type="button" onClick={() => onSearch("", { home: true })}>
          Arxivist
        </button>
        <SearchBox query={query} setQuery={setQuery} onSearch={onSearch} compact loading={loading} />
        <div className="results-actions">
          <button
            className={`text-button ${searchMode === "agent" ? "active" : ""}`}
            type="button"
            onClick={() => onModeChange(searchMode === "agent" ? "traditional" : "agent")}
          >
            AI
          </button>
          <IconButton label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`} pressed={theme === "dark"} onClick={toggleTheme}>
            <ThemeIcon theme={theme} />
          </IconButton>
        </div>
      </header>

      <nav className="tabs" aria-label="Result types">
        <a className={searchMode === "traditional" ? "active" : ""} href="#results" onClick={() => onModeChange("traditional")}>All</a>
        <a href="#results">Papers</a>
        <a href="#results">Books</a>
        <a href="#results">Code</a>
        <a className={searchMode === "agent" ? "active" : ""} href="#results" onClick={() => onModeChange("agent")}>AI</a>
      </nav>

      <section className="filter-row" aria-label="Search filters">
        <span>{status}</span>
        <span>{health}</span>
      </section>

      <section id="results" className="results-shell" aria-live="polite">
        {searchMode === "agent" ? <SearchAssist assist={agentAssist} /> : null}
        {loading ? <StateMessage title="Searching" body="Looking through the current index." /> : null}
        {!loading && message ? <StateMessage title={message.title} body={message.body} tone={message.tone} /> : null}
        {!loading && !message ? (
          <>
            <ol className="result-list">
              {results.map((item) => (
                <ResultItem key={`${item.url}-${item.title}`} item={item} />
              ))}
            </ol>
            {showPagination ? (
              <PaginationFooter
                query={query}
                currentPage={currentPage}
                totalPages={totalPages}
                hasPrevious={hasPrevious}
                hasNext={hasNext}
                loading={loading}
                onSearch={onSearch}
              />
            ) : null}
          </>
        ) : null}
      </section>
    </main>
  );
}

function SearchAssist({ assist }) {
  if (assist.status === "idle") {
    return null;
  }

  if (assist.status === "loading") {
    return (
      <section className="search-assist" aria-label="Search Assist">
        <div className="assist-header">
          <span className="assist-kicker">Search Assist</span>
          <span className="assist-status">Thinking</span>
        </div>
        <p className="assist-loading">Reading the index and preparing an answer.</p>
      </section>
    );
  }

  if (assist.status === "error") {
    return (
      <section className="search-assist" data-tone="error" aria-label="Search Assist">
        <div className="assist-header">
          <span className="assist-kicker">Search Assist</span>
          <span className="assist-status">Unavailable</span>
        </div>
        <p>{assist.error}</p>
      </section>
    );
  }

  const sources = (assist.sources ?? []).slice(0, agentSourceLimit);

  return (
    <section className="search-assist" aria-label="Search Assist">
      <div className="assist-header">
        <span className="assist-kicker">Search Assist</span>
        <span className="assist-status">
          {assist.llmCalls.toLocaleString()} model calls / {assist.toolCallCount.toLocaleString()} tool calls
        </span>
      </div>
      <div className="assist-answer">
        {splitAnswer(assist.answer).map((line, index) => (
          <p key={`${line}-${index}`}>{line}</p>
        ))}
      </div>
      {sources.length > 0 ? (
        <div className="assist-sources" aria-label="Search Assist sources">
          {sources.map((source) => (
            <a key={source.url} href={source.url} target="_blank" rel="noreferrer" title={source.title || source.url}>
              {source.title || sourceLabel(source.url)}
            </a>
          ))}
        </div>
      ) : null}
    </section>
  );
}

function splitAnswer(answer) {
  const lines = String(answer || "").split(/\n+/).map((line) => line.trim()).filter(Boolean);
  return lines.length > 0 ? lines : ["No agent answer was returned."];
}

function sourceLabel(url) {
  try {
    return new URL(url).hostname;
  } catch {
    return url;
  }
}

function PaginationFooter({
  query,
  currentPage,
  totalPages,
  hasPrevious,
  hasNext,
  loading,
  onSearch
}) {
  return (
    <nav className="pagination-footer" aria-label="Search results pages">
      <button
        className="pagination-arrow"
        type="button"
        aria-label="Previous results page"
        title="Previous results page"
        disabled={!hasPrevious || loading}
        onClick={() => onSearch(query, { page: currentPage - 1 })}
      >
        <ArrowIcon direction="previous" />
      </button>
      <span className="pagination-label">Page {currentPage.toLocaleString()} of {totalPages.toLocaleString()}</span>
      <button
        className="pagination-arrow"
        type="button"
        aria-label="Next results page"
        title="Next results page"
        disabled={!hasNext || loading}
        onClick={() => onSearch(query, { page: currentPage + 1 })}
      >
        <ArrowIcon direction="next" />
      </button>
    </nav>
  );
}

function StateMessage({ title, body, tone = "default" }) {
  return (
    <div className="state-message" data-tone={tone}>
      <h2>{title}</h2>
      <p>{body}</p>
    </div>
  );
}

function ResultItem({ item }) {
  return (
    <li className="result-item">
      <a href={item.url} target="_blank" rel="noreferrer">{item.title || item.url}</a>
      <p className="result-url">{item.url}</p>
      <p className="result-snippet">{item.snippet || "No snippet available."}</p>
      <p className="result-meta">
        Score {formatMetric(item.score)} / PageRank {formatMetric(item.page_rank)}
      </p>
    </li>
  );
}

function App() {
  const [theme, setTheme] = useTheme();
  const [query, setQuery] = useState("");
  const [searchMode, setSearchMode] = useState("traditional");
  const [results, setResults] = useState([]);
  const [health, setHealth] = useState("Checking index...");
  const [pageMode, setPageMode] = useState("home");
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState(null);
  const [status, setStatus] = useState("Ready");
  const [currentPage, setCurrentPage] = useState(1);
  const [totalPages, setTotalPages] = useState(0);
  const [totalResults, setTotalResults] = useState(0);
  const [hasPrevious, setHasPrevious] = useState(false);
  const [hasNext, setHasNext] = useState(false);
  const [agentAssist, setAgentAssist] = useState({ status: "idle" });
  const agentRequestId = useRef(0);

  useEffect(() => {
    let active = true;

    async function loadHealth() {
      try {
        const response = await fetch(apiUrl("/health"));
        if (!response.ok) {
          throw new Error(`Health check failed with ${response.status}`);
        }
        const nextHealth = await response.json();
        if (active) {
          setHealth(`${nextHealth.documents.toLocaleString()} pages indexed across ${nextHealth.terms.toLocaleString()} terms`);
        }
      } catch {
        if (active) {
          setHealth(`Index status unavailable at ${apiBaseUrl}`);
        }
      }
    }

    loadHealth();
    return () => {
      active = false;
    };
  }, []);

  async function search(nextQuery, options = {}) {
    const trimmedQuery = nextQuery.trim();
    const requestedMode = options.mode ?? searchMode;
    if (options.home) {
      setPageMode("home");
      setMessage(null);
      setResults([]);
      setStatus("Ready");
      setCurrentPage(1);
      setTotalPages(0);
      setTotalResults(0);
      setHasPrevious(false);
      setHasNext(false);
      agentRequestId.current += 1;
      setAgentAssist({ status: "idle" });
      return;
    }
    if (!trimmedQuery) {
      return;
    }

    const requestedPage = Math.max(1, options.page ?? 1);
    setQuery(trimmedQuery);
    setPageMode("results");
    setLoading(true);
    setMessage(null);
    setResults([]);
    setStatus("Searching...");
    if (requestedMode === "agent" && requestedPage === 1) {
      runAgentSearch(trimmedQuery);
    } else if (requestedMode !== "agent") {
      agentRequestId.current += 1;
      setAgentAssist({ status: "idle" });
    }

    try {
      const response = await fetch(apiUrl("/search"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          query: trimmedQuery,
          page: requestedPage,
          page_size: searchPageSize,
          mode: "traditional"
        })
      });

      if (!response.ok) {
        throw new Error(`Search failed with ${response.status}`);
      }

      const payload = await response.json();
      const nextResults = payload.results ?? [];
      const nextPage = payload.page ?? requestedPage;
      const nextTotalPages = payload.total_pages ?? 0;
      const nextTotalResults = payload.total_results ?? nextResults.length;
      setResults(nextResults);
      setCurrentPage(nextPage);
      setTotalPages(nextTotalPages);
      setTotalResults(nextTotalResults);
      setHasPrevious(Boolean(payload.has_previous));
      setHasNext(Boolean(payload.has_next));
      setStatus(
        nextTotalResults === 0
          ? "No results found"
          : `Page ${nextPage.toLocaleString()} of ${nextTotalPages.toLocaleString()} / ${nextTotalResults.toLocaleString()} results`
      );
      setMessage(
        nextTotalResults === 0
          ? { title: "No results found", body: "Try a broader phrase or check whether the crawler has indexed related pages." }
          : null
      );
    } catch (error) {
      setStatus("Search unavailable");
      setTotalPages(0);
      setTotalResults(0);
      setHasPrevious(false);
      setHasNext(false);
      setMessage({
        title: "Search API unavailable",
        body: error.message,
        tone: "error"
      });
    } finally {
      setLoading(false);
    }
  }

  async function runAgentSearch(nextQuery) {
    const requestId = agentRequestId.current + 1;
    agentRequestId.current = requestId;
    setAgentAssist({ status: "loading" });

    try {
      const response = await fetch(agentApiUrl("/agent/search"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          query: nextQuery,
          top_k: searchPageSize,
          context: agentContext()
        })
      });

      if (!response.ok) {
        throw new Error(`Agent search failed with ${response.status}`);
      }

      const payload = await response.json();
      if (agentRequestId.current !== requestId) {
        return;
      }
      setAgentAssist({
        status: "success",
        answer: payload.answer ?? "",
        sources: payload.sources ?? [],
        toolCallCount: payload.tool_call_count ?? 0,
        llmCalls: payload.llm_calls ?? 0
      });
    } catch (error) {
      if (agentRequestId.current !== requestId) {
        return;
      }
      setAgentAssist({
        status: "error",
        error: error.message
      });
    }
  }

  function agentContext() {
    const now = new Date();
    return {
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      locale: navigator.language,
      local_time: now.toISOString()
    };
  }

  function changeMode(nextMode) {
    setSearchMode(nextMode);
    if (pageMode === "results" && query.trim()) {
      search(query, { mode: nextMode, page: 1 });
    }
  }

  const toggleTheme = () => setTheme((current) => (current === "dark" ? "light" : "dark"));

  if (pageMode === "home") {
    return (
      <SearchHome
        query={query}
        setQuery={setQuery}
        onSearch={search}
        health={health}
        theme={theme}
        toggleTheme={toggleTheme}
        loading={loading}
        searchMode={searchMode}
        onModeChange={changeMode}
      />
    );
  }

  return (
    <SearchResults
      query={query}
      setQuery={setQuery}
      onSearch={search}
      results={results}
      status={status}
      message={message}
      health={health}
      theme={theme}
      toggleTheme={toggleTheme}
      loading={loading}
      currentPage={currentPage}
      totalPages={totalPages}
      totalResults={totalResults}
      hasPrevious={hasPrevious}
      hasNext={hasNext}
      searchMode={searchMode}
      onModeChange={changeMode}
      agentAssist={agentAssist}
    />
  );
}

createRoot(document.getElementById("root")).render(<App />);
