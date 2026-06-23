use arxivist_core::CrawlSkipReason;

pub const MIN_TEXT_CHARS: usize = 80;

pub fn is_html(content_type: &Option<String>, html: &str) -> bool {
    content_type
        .as_deref()
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or_else(|| {
            html.trim_start().starts_with("<!doctype html") || html.contains("<html")
        })
}

pub fn looks_javascript_required(html: &str, extracted_text: &str) -> bool {
    let lower_html = html.to_ascii_lowercase();
    let lower_text = extracted_text.to_ascii_lowercase();
    let script_tags = lower_html.matches("<script").count();
    let text_len = extracted_text.trim().chars().count();

    text_len < MIN_TEXT_CHARS
        && (script_tags >= 3
            || lower_text.contains("enable javascript")
            || lower_text.contains("requires javascript")
            || lower_text.contains("please enable js")
            || lower_html.contains("id=\"__next\"")
            || lower_html.contains("id=\"root\""))
}

pub fn should_penalize(reason: Option<CrawlSkipReason>) -> bool {
    matches!(
        reason,
        Some(
            CrawlSkipReason::NonHtml
                | CrawlSkipReason::EmptyText
                | CrawlSkipReason::LikelyJavascriptRequired
                | CrawlSkipReason::FetchError
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_javascript_required_shells() {
        let html = r#"<html><body><div id="root"></div><script></script><script></script><script></script></body></html>"#;
        assert!(looks_javascript_required(html, ""));
    }

    #[test]
    fn does_not_penalize_robots_txt() {
        assert!(!should_penalize(Some(CrawlSkipReason::RobotsTxt)));
        assert!(should_penalize(Some(
            CrawlSkipReason::LikelyJavascriptRequired
        )));
    }
}
