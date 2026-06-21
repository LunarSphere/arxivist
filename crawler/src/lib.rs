//wrote this wrong in legacy lib.rs should just be pub mods and re exports
pub mod api;
pub mod aws;
pub mod config;
pub mod crawl;
pub mod extract;
pub mod ids;
pub mod models;
pub mod service;
pub mod storage;

pub use api::router;
pub use config::{AwsSettings, ServerSettings, SpiderSettings};
pub use service::CrawlerService;
