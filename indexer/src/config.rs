use std::{env, net::SocketAddr, path::PathBuf};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ServerSettings {
    pub bind_addr: SocketAddr,
}

impl ServerSettings {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8081".to_string());
        Ok(Self {
            bind_addr: bind_addr
                .parse()
                .with_context(|| format!("invalid BIND_ADDR: {bind_addr}"))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub crawler_pages_table: String,
    pub index_builds_table: String,
    pub raw_html_bucket: String,
    pub index_bucket: String,
}

impl AwsSettings {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            crawler_pages_table: required_env("INDEXER_CRAWLER_PAGES_TABLE")?,
            index_builds_table: required_env("INDEXER_BUILDS_TABLE")?,
            raw_html_bucket: required_env("INDEXER_RAW_HTML_BUCKET")?,
            index_bucket: required_env("INDEXER_INDEX_BUCKET")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct IndexerSettings {
    pub work_dir: PathBuf,
    pub min_text_chars: usize,
    pub language_confidence: f64,
    pub pagerank_damping: f64,
    pub pagerank_iterations: usize,
}

impl IndexerSettings {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            work_dir: env::var("INDEXER_WORK_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/tmp/arxivist-indexer")),
            min_text_chars: optional_usize("INDEXER_MIN_TEXT_CHARS", 200)?,
            language_confidence: optional_f64("INDEXER_LANGUAGE_CONFIDENCE", 0.80)?,
            pagerank_damping: optional_f64("INDEXER_PAGERANK_DAMPING", 0.85)?,
            pagerank_iterations: optional_usize("INDEXER_PAGERANK_ITERATIONS", 20)?,
        })
    }
}

fn required_env(key: &str) -> Result<String> {
    let value = env::var(key).with_context(|| format!("{key} must be set"))?;
    if value.trim().is_empty() {
        anyhow::bail!("{key} must not be empty");
    }
    Ok(value)
}

fn optional_usize(key: &str, default: usize) -> Result<usize> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{key} must be an unsigned integer")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}

fn optional_f64(key: &str, default: f64) -> Result<f64> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{key} must be a floating-point number")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}
