import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import "./styles.css";

const config = window.ARXIVIST_CONFIG ?? {};
const configuredApiBaseUrl = (config.apiBaseUrl ?? "").trim();
const apiBaseUrl = (configuredApiBaseUrl || "http://127.0.0.1:3000").replace(/\/$/, "");
const themeStorageKey = "arxivist-theme";

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

function SearchHome({ query, setQuery, onSearch, health, theme, toggleTheme, loading }) {
  return (
    <main className="home-shell">
      <header className="home-actions" aria-label="Page controls">
        <button className="text-button" type="button">AI</button>
        <IconButton label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`} pressed={theme === "dark"} onClick={toggleTheme}>
          <ThemeIcon theme={theme} />
        </IconButton>
        <IconButton label="Menu">Menu</IconButton>
      </header>

      <section className="home-panel" aria-labelledby="page-title">
        <h1 id="page-title" className="brand">Arxivist</h1>
        <div className="mode-switch" aria-label="Search mode">
          <button className="active" type="button">Search</button>
          <button type="button" aria-disabled="true">AI</button>
        </div>
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
  loading
}) {
  return (
    <main className="results-page">
      <header className="results-top">
        <button className="mini-brand" type="button" onClick={() => onSearch("", { home: true })}>
          Arxivist
        </button>
        <SearchBox query={query} setQuery={setQuery} onSearch={onSearch} compact loading={loading} />
        <div className="results-actions">
          <button className="text-button" type="button">AI</button>
          <IconButton label={`Switch to ${theme === "dark" ? "light" : "dark"} mode`} pressed={theme === "dark"} onClick={toggleTheme}>
            <ThemeIcon theme={theme} />
          </IconButton>
        </div>
      </header>

      <nav className="tabs" aria-label="Result types">
        <a className="active" href="#results">All</a>
        <a href="#results">Papers</a>
        <a href="#results">Books</a>
        <a href="#results">Code</a>
        <a href="#results">AI</a>
      </nav>

      <section className="filter-row" aria-label="Search filters">
        <span>{status}</span>
        <span>{health}</span>
      </section>

      <section id="results" className="results-shell" aria-live="polite">
        {loading ? <StateMessage title="Searching" body="Looking through the current index." /> : null}
        {!loading && message ? <StateMessage title={message.title} body={message.body} tone={message.tone} /> : null}
        {!loading && !message ? (
          <ol className="result-list">
            {results.map((item) => (
              <ResultItem key={`${item.url}-${item.title}`} item={item} />
            ))}
          </ol>
        ) : null}
      </section>
    </main>
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
  const [results, setResults] = useState([]);
  const [health, setHealth] = useState("Checking index...");
  const [pageMode, setPageMode] = useState("home");
  const [loading, setLoading] = useState(false);
  const [message, setMessage] = useState(null);
  const [status, setStatus] = useState("Ready");

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
    if (options.home) {
      setPageMode("home");
      setMessage(null);
      setResults([]);
      setStatus("Ready");
      return;
    }
    if (!trimmedQuery) {
      return;
    }

    setQuery(trimmedQuery);
    setPageMode("results");
    setLoading(true);
    setMessage(null);
    setResults([]);
    setStatus("Searching...");

    try {
      const response = await fetch(apiUrl("/search"), {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ query: trimmedQuery, top_k: 10, mode: "traditional" })
      });

      if (!response.ok) {
        throw new Error(`Search failed with ${response.status}`);
      }

      const payload = await response.json();
      const nextResults = payload.results ?? [];
      setResults(nextResults);
      setStatus(
        nextResults.length === 0
          ? "No results found"
          : `About ${nextResults.length.toLocaleString()} results`
      );
      setMessage(
        nextResults.length === 0
          ? { title: "No results found", body: "Try a broader phrase or check whether the crawler has indexed related pages." }
          : null
      );
    } catch (error) {
      setStatus("Search unavailable");
      setMessage({
        title: "Search API unavailable",
        body: error.message,
        tone: "error"
      });
    } finally {
      setLoading(false);
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
    />
  );
}

createRoot(document.getElementById("root")).render(<App />);
