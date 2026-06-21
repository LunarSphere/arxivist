// not commenting all of these
// the logic for parsing the page html lives here
use scraper::{Html, Selector};
use url::Url;

use crate::models::AssetRecord;

const TEXT_PREVIEW_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPage {
    pub title: Option<String>,
    pub links: Vec<String>,
    pub assets: Vec<AssetRecord>,
    pub text_preview: Option<String>,
    pub word_count: usize,
}

pub fn extract_page(base_url: &str, html: &str) -> ExtractedPage {
    let document = Html::parse_document(html);
    let base = Url::parse(base_url).ok();

    let title = first_text(&document, "title");
    let text = document
        .root_element()
        .text()
        .collect::<Vec<_>>()
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let word_count = text.split_whitespace().count();
    let text_preview = trim_preview(&text);

    let links = collect_urls(&document, "a[href]", "href", base.as_ref());
    let mut assets = collect_assets(&document, "img[src]", "src", "image", base.as_ref());
    assets.extend(collect_assets(
        &document,
        "script[src]",
        "src",
        "script",
        base.as_ref(),
    ));
    assets.extend(collect_assets(
        &document,
        r#"link[rel="stylesheet"]"#,
        "href",
        "stylesheet",
        base.as_ref(),
    ));

    ExtractedPage {
        title,
        links,
        assets,
        text_preview,
        word_count,
    }
}

fn first_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|el| el.text().collect::<String>().trim().to_string())
        .filter(|value| !value.is_empty())
}

fn collect_urls(document: &Html, selector: &str, attr: &str, base: Option<&Url>) -> Vec<String> {
    let selector = match Selector::parse(selector) {
        Ok(selector) => selector,
        Err(_) => return Vec::new(),
    };

    let mut urls = document
        .select(&selector)
        .filter_map(|el| el.value().attr(attr))
        .filter_map(|value| absolutize(value, base))
        .collect::<Vec<_>>();
    urls.sort();
    urls.dedup();
    urls
}

fn collect_assets(
    document: &Html,
    selector: &str,
    attr: &str,
    asset_type: &str,
    base: Option<&Url>,
) -> Vec<AssetRecord> {
    let selector = match Selector::parse(selector) {
        Ok(selector) => selector,
        Err(_) => return Vec::new(),
    };

    let mut assets = document
        .select(&selector)
        .filter_map(|el| {
            let asset_url = absolutize(el.value().attr(attr)?, base)?;
            let alt_text = el
                .value()
                .attr("alt")
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty());
            Some(AssetRecord {
                asset_url,
                asset_type: asset_type.to_string(),
                alt_text,
            })
        })
        .collect::<Vec<_>>();

    assets.sort_by(|a, b| {
        a.asset_type
            .cmp(&b.asset_type)
            .then(a.asset_url.cmp(&b.asset_url))
    });
    assets.dedup_by(|a, b| a.asset_type == b.asset_type && a.asset_url == b.asset_url);
    assets
}

fn absolutize(value: &str, base: Option<&Url>) -> Option<String> {
    if let Some(base) = base {
        base.join(value).ok().map(|url| url.to_string())
    } else {
        Url::parse(value).ok().map(|url| url.to_string())
    }
}

fn trim_preview(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    let preview = text.chars().take(TEXT_PREVIEW_CHARS).collect::<String>();
    Some(preview)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_text_links_and_assets() {
        let html = r#"
            <html>
              <head>
                <title>Example Page</title>
                <link rel="stylesheet" href="/site.css">
              </head>
              <body>
                <h1>Hello world</h1>
                <a href="/next">Next</a>
                <img src="/cover.png" alt="Cover">
                <script src="/app.js"></script>
              </body>
            </html>
        "#;

        let page = extract_page("https://example.com/docs/", html);

        assert_eq!(page.title.as_deref(), Some("Example Page"));
        assert!(page.word_count >= 4);
        assert!(page.links.contains(&"https://example.com/next".to_string()));
        assert!(page.assets.iter().any(|asset| {
            asset.asset_type == "image"
                && asset.asset_url == "https://example.com/cover.png"
                && asset.alt_text.as_deref() == Some("Cover")
        }));
        assert!(
            page.assets
                .iter()
                .any(|asset| asset.asset_url == "https://example.com/site.css")
        );
    }
}
