use scraper::{Html, Selector};

const TEXT_PREVIEW_CHARS: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedDocument {
    pub title: Option<String>,
    pub body: String,
    pub text_preview: String,
    pub word_count: usize,
}

pub fn extract_document(html: &str) -> ExtractedDocument {
    let document = Html::parse_document(html);
    let title = first_text(&document, "title");
    let body = body_text(&document);
    let text_preview = body.chars().take(TEXT_PREVIEW_CHARS).collect::<String>();
    let word_count = body.split_whitespace().count();

    ExtractedDocument {
        title,
        body,
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

fn body_text(document: &Html) -> String {
    let body_selector = Selector::parse("body").expect("body selector should be valid");
    let text = document
        .select(&body_selector)
        .next()
        .map(|body| body.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| document.root_element().text().collect::<Vec<_>>().join(" "));

    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_title_body_preview_and_word_count() {
        let doc = extract_document(
            r#"<html><head><title>Example</title></head><body>Hello search world.</body></html>"#,
        );

        assert_eq!(doc.title.as_deref(), Some("Example"));
        assert!(doc.body.contains("Hello search world"));
        assert_eq!(doc.word_count, 3);
        assert!(!doc.text_preview.is_empty());
    }
}
