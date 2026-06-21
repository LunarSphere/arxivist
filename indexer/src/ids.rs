use sha2::{Digest, Sha256};

pub fn sha256_hex(input: impl AsRef<[u8]>) -> String {
    hex_encode(Sha256::digest(input.as_ref()).as_slice())
}

pub fn url_hash(url: &str) -> String {
    sha256_hex(url.as_bytes())
}

pub fn index_s3_prefix(crawl_id: &str, index_build_id: &str) -> String {
    format!("indexes/{crawl_id}/{index_build_id}")
}

pub fn tantivy_s3_prefix(crawl_id: &str, index_build_id: &str) -> String {
    format!("{}/tantivy", index_s3_prefix(crawl_id, index_build_id))
}

pub fn manifest_s3_key(crawl_id: &str, index_build_id: &str) -> String {
    format!(
        "{}/manifest.json",
        index_s3_prefix(crawl_id, index_build_id)
    )
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_index_artifact_paths() {
        assert_eq!(
            index_s3_prefix("crawl-1", "build-1"),
            "indexes/crawl-1/build-1"
        );
        assert_eq!(
            tantivy_s3_prefix("crawl-1", "build-1"),
            "indexes/crawl-1/build-1/tantivy"
        );
        assert_eq!(
            manifest_s3_key("crawl-1", "build-1"),
            "indexes/crawl-1/build-1/manifest.json"
        );
    }
}
