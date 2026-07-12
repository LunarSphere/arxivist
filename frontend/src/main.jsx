import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import "./styles.css";

const config = window.ARXIVIST_CONFIG ?? {};
const configuredApiBaseUrl = (config.apiBaseUrl ?? "").trim();
const apiBaseUrl = (configuredApiBaseUrl || "http://127.0.0.1:3000").replace(/\/$/, "");
const configuredAgentApiBaseUrl = (config.agentApiBaseUrl ?? "").trim();
const agentApiBaseUrl = (configuredAgentApiBaseUrl || "http://127.0.0.1:8000").replace(/\/$/, "");
const themeStorageKey = "arxivist-theme";
const searchPageSize = 10;
const agentSourceLimit = 5;
const mapContextWaitMs = 1500;

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
        <label className="visually-hidden" htmlFor={compact ? "results-query" : "home-query"}>
          Search query
        </label>
        <input
          id={compact ? "results-query" : "home-query"}
          value={query}
          type="search"
          placeholder={compact ? "Search the index..." : "Enter your search query..."}
          autoComplete="off"
          onChange={(event) => setQuery(event.target.value)}
          onFocus={() => setFocused(true)}
          onBlur={() => window.setTimeout(() => setFocused(false), 120)}
        />
      </div>
      <div className="search-controls">
        <button className="search-submit" type="submit" disabled={loading || !query.trim()}>
          ⌕ Search it!
        </button>
        <button className="search-clear" type="button" onClick={() => setQuery("")} disabled={!query}>
          Clear
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
    <nav className="mode-switch" aria-label="Search mode">
      <button
        className={searchMode === "traditional" ? "active" : ""}
        type="button"
        onClick={() => onModeChange("traditional")}
      >
        ⌕ Search
      </button>
      <button
        className={searchMode === "maps" ? "active" : ""}
        type="button"
        onClick={() => onModeChange("maps")}
      >
        ♧ Map
      </button>
    </nav>
  );
}

function AppChrome({ theme, toggleTheme, searchMode, onModeChange, searchAssistEnabled, onSearchAssistToggle }) {
  return (
    <header className="app-chrome">
      <div className="masthead">
        <span aria-hidden="true">◉</span> ARXIVIST SEARCH ENGINE <span className="masthead-version">v2.4</span>
        <div className="chrome-actions">
          <button
            className={`assist-toggle ${searchAssistEnabled ? "active" : ""}`}
            type="button"
            aria-pressed={searchAssistEnabled}
            onClick={onSearchAssistToggle}
          >
            AI {searchAssistEnabled ? "ON" : "OFF"}
          </button>
          <button className="theme-toggle" type="button" onClick={toggleTheme}>
            {theme === "dark" ? "☼ LIGHT" : "☾ DARK"}
          </button>
        </div>
      </div>
      <ModeSwitch searchMode={searchMode} onModeChange={onModeChange} />
    </header>
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
  onModeChange,
  searchAssistEnabled,
  onSearchAssistToggle
}) {
  return (
    <main className="home-shell">
      <AppChrome
        theme={theme}
        toggleTheme={toggleTheme}
        searchMode={searchMode}
        onModeChange={onModeChange}
        searchAssistEnabled={searchAssistEnabled}
        onSearchAssistToggle={onSearchAssistToggle}
      />

      <section className="home-panel" aria-labelledby="page-title">
        <h1 id="page-title" className="brand">{searchMode === "maps" ? "Arxivist Atlas" : "Arxivist"}</h1>
        <p className="home-kicker">{searchMode === "maps" ? "Explore places with research context" : health}</p>
        <SearchBox query={query} setQuery={setQuery} onSearch={onSearch} loading={loading} />
        <p className="tagline">{searchMode === "maps" ? "Search a topic, then locate related places nearby." : "Focused search across your indexed research corpus."}</p>
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
  searchAssistEnabled,
  onSearchAssistToggle,
  agentAssist,
  mapSearch,
  manualLocation,
  setManualLocation,
  mapRadius,
  onRadiusChange
}) {
  const showPagination = totalResults > 0 && totalPages > 0 && !message;

  return (
    <main className="results-page">
      <AppChrome
        theme={theme}
        toggleTheme={toggleTheme}
        searchMode={searchMode}
        onModeChange={onModeChange}
        searchAssistEnabled={searchAssistEnabled}
        onSearchAssistToggle={onSearchAssistToggle}
      />

      <section className={`results-hero ${searchMode === "maps" ? "atlas-hero" : ""}`}>
        <button className="mini-brand" type="button" onClick={() => onSearch("", { home: true })}>
          {searchMode === "maps" ? "Arxivist Atlas" : "Arxivist"}
        </button>
        {searchMode === "maps" ? <p>Explore places with research context</p> : null}
        <SearchBox query={query} setQuery={setQuery} onSearch={onSearch} compact loading={loading} />
      </section>

      {searchMode === "traditional" ? (
        <section className="filter-row" aria-label="Search status">
          <span>{status}</span>
          <span>{health}</span>
        </section>
      ) : null}

      <section id="results" className={`results-shell ${searchMode === "maps" ? "map-results-shell" : ""}`} aria-live="polite">
        {searchMode === "traditional" && searchAssistEnabled ? <SearchAssist assist={agentAssist} /> : null}
        {searchMode === "maps" ? (
          <MapSearch
            search={query}
            mapSearch={mapSearch}
            manualLocation={manualLocation}
            setManualLocation={setManualLocation}
            mapRadius={mapRadius}
            onRadiusChange={onRadiusChange}
            onSearch={onSearch}
          />
        ) : null}
        {searchMode !== "maps" && loading ? <StateMessage title="Searching" body="Looking through the current index." /> : null}
        {searchMode !== "maps" && !loading && message ? <StateMessage title={message.title} body={message.body} tone={message.tone} /> : null}
        {searchMode !== "maps" && !loading && !message ? (
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

function MapSearch({
  search,
  mapSearch,
  manualLocation,
  setManualLocation,
  mapRadius,
  onRadiusChange,
  onSearch
}) {
  return (
    <section className="map-search" aria-label="Nearby map search">
      <div className="map-controls">
        <div className="map-location-field">
          <label htmlFor="map-location">Location</label>
          <input
            id="map-location"
            value={manualLocation}
            onChange={(event) => setManualLocation(event.target.value)}
            placeholder="Use my location or enter a city, address, or ZIP"
          />
        </div>
        <label className="map-radius-field" htmlFor="map-radius">
          Radius
          <select id="map-radius" value={mapRadius} onChange={(event) => onRadiusChange(Number(event.target.value))}>
            <option value={1000}>1 km</option>
            <option value={5000}>5 km</option>
            <option value={10000}>10 km</option>
          </select>
        </label>
        <button className="map-search-button" type="button" onClick={() => onSearch(search, { mode: "maps" })} disabled={!search.trim()}>
          Search nearby
        </button>
      </div>

      {mapSearch.status === "loading" ? <StateMessage title="Finding nearby places" body="Resolving your location and querying OpenStreetMap." /> : null}
      {mapSearch.status === "location-required" ? (
        <StateMessage title="Location needed" body="Allow location access or enter a city, address, or ZIP code above." />
      ) : null}
      {mapSearch.status === "error" ? <StateMessage title="Map search unavailable" body={mapSearch.error} tone="error" /> : null}
      {mapSearch.status === "success" ? <PlaceMap data={mapSearch.data} /> : null}
    </section>
  );
}

function PlaceMap({ data }) {
  const mapElement = useRef(null);
  const mapInstance = useRef(null);
  const markerLayer = useRef(null);
  const markers = useRef([]);

  useEffect(() => {
    if (!mapElement.current) {
      return undefined;
    }
    const map = L.map(mapElement.current, { scrollWheelZoom: true });
    L.tileLayer("https://{s}.tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 19,
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright" target="_blank" rel="noreferrer">OpenStreetMap contributors</a>'
    }).addTo(map);
    markerLayer.current = L.layerGroup().addTo(map);
    mapInstance.current = map;
    return () => {
      map.remove();
      mapInstance.current = null;
      markerLayer.current = null;
    };
  }, []);

  useEffect(() => {
    const map = mapInstance.current;
    const layer = markerLayer.current;
    if (!map || !layer || !data) {
      return;
    }
    layer.clearLayers();
    markers.current = [];
    const center = [data.center.latitude, data.center.longitude];
    const bounds = L.latLngBounds(center, center);
    L.circle(center, { radius: data.radius_m, color: "#65b87a", fillOpacity: 0.06, weight: 1 }).addTo(layer);

    data.places.forEach((place, index) => {
      const point = [place.latitude, place.longitude];
      bounds.extend(point);
      const marker = L.marker(point, {
        icon: L.divIcon({
          className: "map-marker",
          html: String(index + 1),
          iconSize: [28, 28],
          iconAnchor: [14, 14]
        })
      }).addTo(layer);
      marker.bindPopup(`<strong>${escapeHtml(place.name)}</strong><br>${escapeHtml(place.category)}`);
      markers.current[index] = marker;
    });
    map.fitBounds(bounds, { padding: [30, 30], maxZoom: 15 });
    window.setTimeout(() => map.invalidateSize(), 0);
  }, [data]);

  function focusPlace(index) {
    const place = data.places[index];
    const marker = markers.current[index];
    if (!place || !marker || !mapInstance.current) {
      return;
    }
    mapInstance.current.setView([place.latitude, place.longitude], Math.max(mapInstance.current.getZoom(), 15));
    marker.openPopup();
  }

  return (
    <div className="map-result-layout">
      <div ref={mapElement} className="map-canvas" aria-label="OpenStreetMap results map" />
      <div className="place-list">
        <div className="place-list-header">
          <span>{data.places.length.toLocaleString()} places</span>
          <span>{data.source === "nominatim-fallback" ? "OSM fallback" : (data.center.label || "Search area")}</span>
        </div>
        {data.places.length === 0 ? <p className="place-empty">No mapped places matched this search.</p> : null}
        {data.places.map((place, index) => (
          <div
            className="place-item"
            key={place.id}
            role="button"
            tabIndex={0}
            onClick={() => focusPlace(index)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                focusPlace(index);
              }
            }}
          >
            <span className="place-number">{index + 1}</span>
            <div>
              <h2>{place.name}</h2>
              <p>
                {place.category}
                {place.distance_m != null ? ` · ${formatDistance(place.distance_m)}` : ""}
                {place.address ? ` · ${place.address}` : ""}
              </p>
              {place.website ? <a href={place.website} target="_blank" rel="noreferrer">Website</a> : null}
            </div>
          </div>
        ))}
        <p className="map-attribution">{data.attribution}</p>
      </div>
    </div>
  );
}

function escapeHtml(value) {
  return String(value).replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "'": "&#39;",
    '"': "&quot;"
  })[character]);
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

function formatDistance(distanceMeters) {
  if (!Number.isFinite(distanceMeters)) {
    return "";
  }
  return distanceMeters < 1000
    ? `${Math.round(distanceMeters)} m`
    : `${(distanceMeters / 1000).toFixed(1)} km`;
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
  const [searchAssistEnabled, setSearchAssistEnabled] = useState(false);
  const [agentAssist, setAgentAssist] = useState({ status: "idle" });
  const [mapSearch, setMapSearch] = useState({ status: "idle" });
  const [manualLocation, setManualLocation] = useState("");
  const [mapRadius, setMapRadius] = useState(5000);
  const agentRequestId = useRef(0);
  const mapRequestId = useRef(0);
  const searchCache = useRef(new Map());
  const inFlightRequests = useRef(new Map());
  const browserLocation = useRef(null);
  const browserLocationRequest = useRef(null);

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

  function cacheKey(...parts) {
    return JSON.stringify(parts);
  }

  function requestOnce(key, request) {
    const pending = inFlightRequests.current.get(key);
    if (pending) {
      return pending;
    }
    const nextRequest = Promise.resolve().then(request).finally(() => {
      if (inFlightRequests.current.get(key) === nextRequest) {
        inFlightRequests.current.delete(key);
      }
    });
    inFlightRequests.current.set(key, nextRequest);
    return nextRequest;
  }

  function getBrowserLocation() {
    if (browserLocation.current) {
      return Promise.resolve(browserLocation.current);
    }
    if (!navigator.geolocation) {
      return Promise.resolve(null);
    }
    if (!browserLocationRequest.current) {
      browserLocationRequest.current = new Promise((resolve, reject) => {
        navigator.geolocation.getCurrentPosition(resolve, reject, {
          enableHighAccuracy: false,
          timeout: 8000,
          maximumAge: 300000
        });
      }).then((position) => {
        browserLocation.current = {
          latitude: position.coords.latitude,
          longitude: position.coords.longitude
        };
        return browserLocation.current;
      }).catch(() => null).finally(() => {
        browserLocationRequest.current = null;
      });
    }
    return browserLocationRequest.current;
  }

  function applyTraditionalPayload(payload, requestedPage) {
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
  }

  function isLocalIntent(nextQuery) {
    return /\b(near me|nearby|restaurant|restaurants|burger|burgers|coffee|cafe|cafes|bar|pub|brewery|library|libraries|park|parks|hotel|hotels|store|stores|shop|shops|food)\b/i.test(nextQuery);
  }

  function prefetchRelatedSearches(nextQuery, requestedMode, requestedRadius, includeAssist) {
    const shouldFetchMap = requestedMode === "maps" || isLocalIntent(nextQuery);
    if (!shouldFetchMap) {
      if (includeAssist) {
        runAgentSearch(nextQuery, null);
      }
      return;
    }

    if (!includeAssist) {
      runMapSearch(nextQuery, requestedRadius);
      return;
    }

    let agentStarted = false;
    function startAgent(mapResult) {
      if (agentStarted) {
        return;
      }
      agentStarted = true;
      runAgentSearch(nextQuery, mapResult?.data ?? null, mapResult?.location ?? null);
    }

    runMapSearch(nextQuery, requestedRadius).then(startAgent);
    window.setTimeout(() => {
      startAgent({ data: null, location: browserLocation.current });
    }, mapContextWaitMs);
  }

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
    prefetchRelatedSearches(
      trimmedQuery,
      requestedMode,
      options.radius ?? mapRadius,
      searchAssistEnabled
    );

    try {
      const traditionalKey = cacheKey("traditional", trimmedQuery, requestedPage);
      const cachedTraditional = searchCache.current.get(traditionalKey);
      if (cachedTraditional) {
        applyTraditionalPayload(cachedTraditional, requestedPage);
        return;
      }

      const payload = await requestOnce(traditionalKey, async () => {
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
        return response.json();
      });
      searchCache.current.set(traditionalKey, payload);
      applyTraditionalPayload(payload, requestedPage);
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

  async function runAgentSearch(nextQuery, mapData, locationContext = null) {
    const requestId = agentRequestId.current + 1;
    agentRequestId.current = requestId;
    setAgentAssist({ status: "loading" });

    const mapContextKey = mapData
      ? cacheKey(
        mapData.center?.latitude,
        mapData.center?.longitude,
        (mapData.places ?? []).map((place) => place.id)
      )
      : locationContext
        ? cacheKey(locationContext.latitude, locationContext.longitude)
        : "none";
    const agentKey = cacheKey("agent", nextQuery.trim(), searchPageSize, mapContextKey);
    const cachedAgent = searchCache.current.get(agentKey);
    if (cachedAgent) {
      setAgentAssist({
        status: "success",
        answer: cachedAgent.answer ?? "",
        sources: cachedAgent.sources ?? [],
        toolCallCount: cachedAgent.tool_call_count ?? 0,
        llmCalls: cachedAgent.llm_calls ?? 0
      });
      return;
    }

    try {
      const payload = await requestOnce(agentKey, async () => {
        const response = await fetch(agentApiUrl("/agent/search"), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            query: nextQuery,
            top_k: searchPageSize,
            context: agentContext(mapData, locationContext),
            nearby_places: mapData?.places ?? []
          })
        });
        if (!response.ok) {
          throw new Error(`Agent search failed with ${response.status}`);
        }
        return response.json();
      });
      if (agentRequestId.current !== requestId) {
        return;
      }
      searchCache.current.set(agentKey, payload);
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

  async function runMapSearch(nextQuery, radius = mapRadius) {
    const requestId = mapRequestId.current + 1;
    mapRequestId.current = requestId;
    setMapRadius(radius);
    setMapSearch({ status: "loading" });

    const location = manualLocation.trim();
    const mapKey = cacheKey("maps", nextQuery.trim(), radius, location);
    const cachedMap = searchCache.current.get(mapKey);
    if (cachedMap) {
      setMapSearch({ status: "success", data: cachedMap });
      return { data: cachedMap, location: cachedMap.center };
    }

    const position = location ? null : await getBrowserLocation();

    if (!position && !location) {
      if (mapRequestId.current === requestId) {
        setMapSearch({ status: "location-required" });
      }
      return { data: null, location: null };
    }

    const body = {
      query: nextQuery,
      radius_m: radius,
      limit: 50
    };
    if (position) {
      body.latitude = position.latitude;
      body.longitude = position.longitude;
    } else {
      body.location = location;
    }

    const mapRequestKey = cacheKey(
      mapKey,
      position?.latitude,
      position?.longitude
    );
    try {
      const payload = await requestOnce(mapRequestKey, async () => {
        const response = await fetch(agentApiUrl("/places/search"), {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(body)
        });
        if (!response.ok) {
          const errorPayload = await response.json().catch(() => ({}));
          throw new Error(errorPayload.detail || `Map search failed with ${response.status}`);
        }
        return response.json();
      });
      if (mapRequestId.current === requestId) {
        searchCache.current.set(mapKey, payload);
        setMapSearch({ status: "success", data: payload });
      }
      return { data: payload, location: payload.center };
    } catch (error) {
      if (mapRequestId.current === requestId) {
        setMapSearch({ status: "error", error: error.message });
      }
      return {
        data: null,
        location: position
      };
    }
  }

  function agentContext(mapData, locationContext) {
    const now = new Date();
    const context = {
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      locale: navigator.language,
      local_time: now.toISOString()
    };
    const center = mapData?.center ?? locationContext;
    if (center) {
      context.latitude = center.latitude;
      context.longitude = center.longitude;
      context.approx_location = center.label;
    }
    return context;
  }

  function changeMode(nextMode) {
    setSearchMode(nextMode);
    if (pageMode === "results" && query.trim()) {
      search(query, { mode: nextMode, page: 1 });
    }
  }

  function toggleSearchAssist() {
    const nextEnabled = !searchAssistEnabled;
    setSearchAssistEnabled(nextEnabled);
    if (!nextEnabled) {
      agentRequestId.current += 1;
      setAgentAssist({ status: "idle" });
      return;
    }
    if (pageMode === "results" && query.trim()) {
      prefetchRelatedSearches(query, searchMode, mapRadius, true);
    }
  }

  function changeMapRadius(nextRadius) {
    setMapRadius(nextRadius);
    if (pageMode === "results" && query.trim()) {
      search(query, { mode: "maps", page: 1, radius: nextRadius });
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
        searchAssistEnabled={searchAssistEnabled}
        onSearchAssistToggle={toggleSearchAssist}
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
      searchAssistEnabled={searchAssistEnabled}
      onSearchAssistToggle={toggleSearchAssist}
      agentAssist={agentAssist}
      mapSearch={mapSearch}
      manualLocation={manualLocation}
      setManualLocation={setManualLocation}
      mapRadius={mapRadius}
      onRadiusChange={changeMapRadius}
    />
  );
}

createRoot(document.getElementById("root")).render(<App />);
