// filter websites for undesried traits
use arxivist_core::CrawlSkipReason;
use scraper::{Html, Selector};
use url::Url;

pub const MIN_TEXT_CHARS: usize = 80;
const ENGLISH_LANGUAGE_PREFIX: &str = "en"; // ensure pages are in english
const WIKIMEDIA_LANGUAGE_PROJECTS: &[&str] = &[
    "wikipedia",
    "wikibooks",
    "wikinews",
    "wikiquote",
    "wikisource",
    "wikiversity",
    "wikivoyage",
    "wiktionary",
];

// check if content is an html page
pub fn is_html(content_type: &Option<String>, html: &str) -> bool {
    content_type
        .as_deref()
        .map(|value| value.to_ascii_lowercase().contains("text/html"))
        .unwrap_or_else(|| {
            html.trim_start().starts_with("<!doctype html") || html.contains("<html")
        })
}

// we dont want pages that requrie javascript
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

// boolean denoting if a page is allowed for crawling
pub fn is_allowed_crawl_url(url: &Url) -> bool {
    let Some(host) = url.host_str().map(|host| host.to_ascii_lowercase()) else {
        return false;
    };

    is_allowed_wikimedia_host(&host)
}

pub fn is_explicitly_non_english(document: &Html) -> bool {
    document_language(document)
        .as_deref()
        .is_some_and(|language| !is_english_language_tag(language))
}

// records reasosn we might skip a page.
pub fn should_penalize(reason: Option<CrawlSkipReason>) -> bool {
    matches!(
        reason,
        Some(
            CrawlSkipReason::NonHtml
                | CrawlSkipReason::EmptyText
                | CrawlSkipReason::LikelyJavascriptRequired
                | CrawlSkipReason::NonEnglish
                | CrawlSkipReason::FetchError
        )
    )
}

// check if the wikapedia page is an allowed project and language
fn is_allowed_wikimedia_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 3 {
        return true;
    }

    let project = labels[labels.len() - 2];
    let tld = labels[labels.len() - 1];
    if tld != "org" || !WIKIMEDIA_LANGUAGE_PROJECTS.contains(&project) {
        // allowed wiki project
        return true;
    }

    let language_label = labels[0];
    language_label == ENGLISH_LANGUAGE_PREFIX || language_label == "www" // allowed language
}

// hopefully returns a string containing the language of the page.
fn document_language(document: &Html) -> Option<String> {
    let selector = Selector::parse("html").expect("static selector is valid");
    document
        .select(&selector)
        .next()
        .and_then(|node| {
            node.value()
                .attr("lang")
                .or_else(|| node.value().attr("xml:lang"))
        })
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(str::to_owned)
}

// check to see if the string contains the english language prefix
fn is_english_language_tag(language: &str) -> bool {
    let normalized = language.trim().to_ascii_lowercase();
    normalized == ENGLISH_LANGUAGE_PREFIX
        || normalized
            .strip_prefix(ENGLISH_LANGUAGE_PREFIX)
            .is_some_and(|rest| rest.starts_with('-') || rest.starts_with('_'))
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

    #[test]
    fn allows_english_wikipedia_urls() {
        assert!(is_allowed_crawl_url(
            &Url::parse("https://en.wikipedia.org/wiki/Main_Page").unwrap()
        ));
        assert!(is_allowed_crawl_url(
            &Url::parse("https://en.m.wikipedia.org/wiki/Rust").unwrap()
        ));
    }

    #[test]
    fn rejects_non_english_wikimedia_language_projects() {
        assert!(!is_allowed_crawl_url(
            &Url::parse("https://fr.wikipedia.org/wiki/Rouille").unwrap()
        ));
        assert!(!is_allowed_crawl_url(
            &Url::parse("https://es.wiktionary.org/wiki/hola").unwrap()
        ));
    }

    #[test]
    fn allows_non_wikimedia_hosts() {
        assert!(is_allowed_crawl_url(
            &Url::parse("https://developer.mozilla.org/en-US/").unwrap()
        ));
    }

    #[test]
    fn detects_explicit_non_english_html_language() {
        assert!(!is_explicitly_non_english(&Html::parse_document(
            r#"<html><body>Hello</body></html>"#
        )));
        assert!(!is_explicitly_non_english(&Html::parse_document(
            r#"<html lang="en-US"><body>Hello</body></html>"#
        )));
        assert!(is_explicitly_non_english(&Html::parse_document(
            r#"<html lang="fr"><body>Bonjour</body></html>"#
        )));
    }
}
