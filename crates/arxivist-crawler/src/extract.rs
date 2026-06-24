// code for parsing html
use scraper::{Html, Selector};
use std::collections::HashSet;
use url::Url;

// return title of html page as string if avail
pub fn title(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .next()
        .map(|node| node.text().collect::<String>().trim().to_owned())
        .filter(|title| !title.is_empty())
}

//return body of html page as a string
pub fn text(document: &Html) -> String {
    let body_selector = Selector::parse("body").expect("static selector is valid");
    let source = document
        .select(&body_selector)
        .next()
        .map(|body| body.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| document.root_element().text().collect::<Vec<_>>().join(" "));

    source.split_whitespace().collect::<Vec<_>>().join(" ")
}

// return list of links from html page
pub fn links(base: &Url, document: &Html) -> Vec<Url> {
    let selector = Selector::parse("a[href]").expect("static selector is valid");
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    for node in document.select(&selector) {
        let Some(href) = node.value().attr("href") else {
            continue;
        };
        let Ok(link) = base.join(href) else {
            continue;
        };
        if matches!(link.scheme(), "http" | "https") && seen.insert(link.as_str().to_owned()) {
            links.push(link);
        }
    }

    links
}
