use std::path::Path;

use anyhow::Result;
use tantivy::{
    Index, doc,
    schema::{FAST, IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions},
    tokenizer::{
        Language, LowerCaser, RemoveLongFilter, SimpleTokenizer, Stemmer, StopWordFilter,
        TextAnalyzer,
    },
};

use crate::models::IndexedPage;

pub const INDEX_TOKENIZER: &str = "arxivist_en";
pub const LEXICAL_RANKING: &str = "tantivy_bm25";
pub const TANTIVY_VERSION: &str = "0.26.1";

#[derive(Debug, Clone)]
pub struct IndexFields {
    pub crawl_id: tantivy::schema::Field,
    pub url_hash: tantivy::schema::Field,
    pub url: tantivy::schema::Field,
    pub title: tantivy::schema::Field,
    pub body: tantivy::schema::Field,
    pub text_preview: tantivy::schema::Field,
    pub s3_key: tantivy::schema::Field,
    pub content_hash: tantivy::schema::Field,
    pub page_rank: tantivy::schema::Field,
    pub word_count: tantivy::schema::Field,
    pub indexed_at: tantivy::schema::Field,
}

#[derive(Debug, Clone)]
pub struct IndexSchema {
    pub schema: Schema,
    pub fields: IndexFields,
}

pub fn build_schema() -> IndexSchema {
    let mut schema_builder = Schema::builder();

    let text_indexing = TextFieldIndexing::default()
        .set_tokenizer(INDEX_TOKENIZER)
        .set_index_option(IndexRecordOption::WithFreqsAndPositions);
    let text_options = TextOptions::default()
        .set_indexing_options(text_indexing)
        .set_stored();

    let crawl_id = schema_builder.add_text_field("crawl_id", STRING | STORED);
    let url_hash = schema_builder.add_text_field("url_hash", STRING | STORED);
    let url = schema_builder.add_text_field("url", STRING | STORED);
    let title = schema_builder.add_text_field("title", text_options.clone());
    let body = schema_builder.add_text_field("body", text_options);
    let text_preview = schema_builder.add_text_field("text_preview", STORED);
    let s3_key = schema_builder.add_text_field("s3_key", STRING | STORED);
    let content_hash = schema_builder.add_text_field("content_hash", STRING | STORED);
    let page_rank = schema_builder.add_f64_field("page_rank", FAST | STORED);
    let word_count = schema_builder.add_u64_field("word_count", FAST | STORED);
    let indexed_at = schema_builder.add_text_field("indexed_at", STRING | STORED);
    let schema = schema_builder.build();

    IndexSchema {
        schema,
        fields: IndexFields {
            crawl_id,
            url_hash,
            url,
            title,
            body,
            text_preview,
            s3_key,
            content_hash,
            page_rank,
            word_count,
            indexed_at,
        },
    }
}

pub fn write_tantivy_index(
    index_dir: &Path,
    pages: &[IndexedPage],
    indexed_at: &str,
) -> Result<()> {
    std::fs::create_dir_all(index_dir)?;
    let index_schema = build_schema();
    let index = Index::create_in_dir(index_dir, index_schema.schema.clone())?;
    register_tokenizer(&index);

    let mut writer = index.writer(50_000_000)?;
    for page in pages {
        writer.add_document(doc!(
            index_schema.fields.crawl_id => page.crawl_id.clone(),
            index_schema.fields.url_hash => page.url_hash.clone(),
            index_schema.fields.url => page.url.clone(),
            index_schema.fields.title => page.title.clone().unwrap_or_default(),
            index_schema.fields.body => page.body.clone(),
            index_schema.fields.text_preview => page.text_preview.clone(),
            index_schema.fields.s3_key => page.s3_key.clone(),
            index_schema.fields.content_hash => page.content_hash.clone().unwrap_or_default(),
            index_schema.fields.page_rank => page.page_rank,
            index_schema.fields.word_count => page.word_count as u64,
            index_schema.fields.indexed_at => indexed_at.to_string(),
        ))?;
    }

    writer.commit()?;
    writer.wait_merging_threads()?;
    Ok(())
}

fn register_tokenizer(index: &Index) {
    let stop_words =
        StopWordFilter::new(Language::English).expect("tantivy should provide English stop words");
    let analyzer = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(stop_words)
        .filter(Stemmer::new(Language::English))
        .build();

    index.tokenizers().register(INDEX_TOKENIZER, analyzer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_index_directory() {
        let tempdir = tempfile::tempdir().unwrap();
        let page = IndexedPage {
            crawl_id: "crawl".to_string(),
            url_hash: "hash".to_string(),
            url: "https://example.com".to_string(),
            title: Some("Example".to_string()),
            body: "This is an English document about search indexing.".to_string(),
            text_preview: "This is an English document about search indexing.".to_string(),
            s3_key: "raw.html".to_string(),
            content_hash: Some("content".to_string()),
            links: Vec::new(),
            word_count: 8,
            page_rank: 1.0,
        };

        write_tantivy_index(tempdir.path(), &[page], "2026-01-01T00:00:00Z").unwrap();

        assert!(tempdir.path().join("meta.json").exists());
    }
}
