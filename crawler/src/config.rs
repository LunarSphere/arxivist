// configure spider, configure AWS

use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ServerSettings {
    pub bind_addr: SocketAddr,
}

impl ServerSettings {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        Ok(Self {
            bind_addr: bind_addr
                .parse()
                .with_context(|| format!("invalid BIND_ADDR: {bind_addr}"))?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct AwsSettings {
    pub s3_bucket: String,
    pub crawls_table: String,
    pub pages_table: String,
}

impl AwsSettings {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            s3_bucket: required_env("CRAWLER_S3_BUCKET")?,
            crawls_table: required_env("CRAWLER_DDB_CRAWLS_TABLE")?,
            pages_table: required_env("CRAWLER_DDB_PAGES_TABLE")?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SpiderSettings {
    pub user_agent: String,
    pub request_timeout: Duration,
    pub crawl_timeout: Duration,
    pub delay_ms: u64,
}

impl SpiderSettings {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            user_agent: env::var("CRAWLER_USER_AGENT")
                .unwrap_or_else(|_| "ArxivistCrawler/0.1".to_string()),
            request_timeout: Duration::from_secs(optional_u64("CRAWLER_REQUEST_TIMEOUT_SECS", 20)?),
            crawl_timeout: Duration::from_secs(optional_u64("CRAWLER_CRAWL_TIMEOUT_SECS", 900)?),
            delay_ms: optional_u64("CRAWLER_DELAY_MS", 250)?,
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

fn optional_u64(key: &str, default: u64) -> Result<u64> {
    match env::var(key) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{key} must be an unsigned integer")),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("failed reading {key}")),
    }
}
