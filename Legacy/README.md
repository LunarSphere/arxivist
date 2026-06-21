# Search Engine (Legacy)
## About

This folder contains the original implementation of my search engine.

The legacy system includes:
- an asynchronous Rust web crawler
- a SQLite-backed page database
- a basic indexing pipeline
- TF-IDF-style ranking
- PageRank computation
- an interactive terminal UI for searching crawled pages

## Setup
Make sure Rust is installed before running the project.
From the `legacy/` directory, run each component in order.
### 1. Crawl pages

```bash
cd webcrawler
cargo run
```

This crawls pages starting from the configured seed URLs and builds the local database. 
The crawler should terminate on its own once the crawl frontier is exhausted or the configured crawl limits are reached.

### 2. Build the index

```bash
cd ../indexer
cargo run
```

This reads the crawled page data and builds the search index used by the query engine.

### 3. Run the terminal search UI

```bash
cd ../tui
cargo run
```

This launches an interactive terminal UI where you can search over the pages collected by the crawler.

## What I Learned

### Web Crawling

At a basic level, a web crawler does the following:
1. Start from one or more seed URLs.
2. Visit a page.
3. Extract hyperlinks from that page.
4. Store newly discovered links in a frontier.
5. Pop the next URL from the frontier.
6. Repeat until the frontier is empty or the crawler reaches a stopping condition.

### Terminal UI Design
I learned basic tui designed patterns and code structure
- application state | app.rs
- application logic | main.rs
- UI rendering components | ui.rs
### Basic Database Design

I used SQL to store crawled pages, extracted text, links, and index data. This helped me understand how a search engine needs different data representations for:

- raw crawled content
- page metadata
- page-to-page links
- terms
- postings
- ranking features

### TF-IDF
- **Term Frequency** measures how often a term appears in a document.
- **Inverse Document Frequency** measures how rare or common a term is across the corpus.
- Terms that appear frequently in a document but rarely across the full corpus receive higher weight.

The metric is just the product of the frequncies.

### PageRank
I also implemented a simple version of PageRank.

The basic idea is:
1. Treat crawled pages as nodes in a graph.
2. Treat hyperlinks between pages as directed edges.
3. Initialize each page with rank `1 / N`, where `N` is the number of pages.
4. Repeatedly update each page's rank based on the rank passed to it by incoming links.
5. Divide each linking page's contribution by the number of outgoing links from that page.
6. Redistribute rank from pages with no outgoing links.
7. Repeat until the ranks stabilize.
8. Optionally use a damping factor to model the probability that a user stops following links or jumps to another page.

This helped me understand how link structure can be used as a ranking signal alongside text relevance.

### Async Rust

The crawler also helped me practice asynchronous Rust with Tokio.
I learned how to structure concurrent crawling work, manage asynchronous requests, and coordinate crawling tasks without blocking the entire program.

## Known Limitations
- tokenization is basic
- ranking is relatively simple
- the crawler is not distributed
- the URL frontier is local and in-memory
- SQLite is used as the main storage layer
- there is no production-grade rate limiting
- there is no cloud deployment
- query latency and indexing performance are not optimized for large corpora
