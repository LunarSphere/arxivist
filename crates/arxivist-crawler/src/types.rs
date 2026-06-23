use url::Url;

#[derive(Debug, Clone)]
pub struct QueueItem {
    pub url: Url,
    pub source_seed: Url,
    pub referrer: Option<Url>,
    pub depth: usize,
}

#[derive(Debug)]
pub struct PageSnapshot {
    pub final_url: Url,
    pub status: u16,
    pub content_type: Option<String>,
    pub html: String,
}
