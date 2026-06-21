use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use url::Url;

pub fn normalize_url(input: &str) -> Result<String> {
    let mut url = Url::parse(input).with_context(|| format!("invalid URL: {input}"))?;
    match url.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!("unsupported URL scheme: {scheme}"),
    }
    if url.host_str().is_none() {
        anyhow::bail!("URL must include a host");
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

pub fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(input.as_ref()))
}

pub fn url_hash(url: &str) -> String {
    sha256_hex(url.as_bytes())
}

pub fn raw_html_s3_key(crawl_id: &str, url_hash: &str) -> String {
    format!("{crawl_id}/raw/{url_hash}.html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_fragments_when_normalizing() {
        let normalized = normalize_url("https://example.com/docs#section").unwrap();
        assert_eq!(normalized, "https://example.com/docs");
    }

    #[test]
    fn rejects_non_http_urls() {
        let err = normalize_url("file:///tmp/page.html").unwrap_err();
        assert!(err.to_string().contains("unsupported URL scheme"));
    }

    #[test]
    fn builds_stable_raw_s3_key() {
        assert_eq!(
            raw_html_s3_key("crawl-1", "abc123"),
            "crawl-1/raw/abc123.html"
        );
    }
}
