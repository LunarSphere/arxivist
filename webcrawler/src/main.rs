/* 
* I want to build a web crawler in rust, 
* dependenceies I'll need tokio, reqwest, scraper, url 
* web crawler process
* start with seed url, fetch page, parse html, extract links, and content, add new urls to queue, repeat
* respect robots.txt, we are crawling and scraping, crawliing is IO-bound so we need tokio for concurrency 
* 
 */
// use thiserror::Error;
use select::document::Document;
use select::predicate::Name;
use url::Url;
use std::collections::VecDeque;
use std::collections::HashSet;
use anyhow::Result;
use dashmap::DashSet; //we use this b/c its safe for multiple threads
use governor::{Quota, RateLimiter};
use reqwest::Client;
use scraper::{Html, Selector};
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::mpsc;


//TODO: add error handling, 
// respect robots.txt ->
// add delay between requests, 
// limit crawl depth, 
// handle dynamic content, 
// add user-agent header, 
// implement concurrency with tokio tasks. 

//some pages are dynamic. the load their content with JS after page loads. 
// if we fetch a site like this with request we'll get bare htlm. because reqwest only makes html request
// to load the bage we need the API, a headless browserlike (selinium), scraping service. 

//bassically an object that represents a single crawler
struct CrawlerState{
    //track client visited limiter
    client:Client,
    visited:DashSet<String>,
    limiter: RateLimiter<
        governor::state::NotKeyed,
        governor::state::InMemoryState,
        governor::clock::DefaultClock,
    >
}

impl CrawlerState{
    //Todo a fn for rquests per second
    fn new(requests_per_second: u32) -> Self{
        let client = Client::builder()
            .user_agent("")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("failed to build HTTP client");
        let quota = Quota::per_second(NonZeroU32::new(requests_per_second).unwrap());
        let limiter = RateLimiter::direct(quota);
        Self{
            client,
            visited: DashSet::new(),
            limiter,
        }
    }
}




// collect urls can fail so we must update the function signature to return an error
async fn collecturls(seed: &str) -> Result<Vec<String>, Box<dyn std::error::Error>>{
    let base_url = Url::parse(seed)?;

    //grab body of the urlpage
    let body = reqwest::get(base_url.as_str())
    .await? //wait for the page to respond
    .text() //process information as text. 
    .await?;

    // list of links
    let links: Vec<String> = Document::from(body.as_str())
      .find(Name("a"))
      .filter_map(|node| node.attr("href"))
      .filter_map(|href| {
        if href.starts_with('#') || href.is_empty(){
            return None;
        }
        base_url.join(href).ok().map(|url| url.to_string())
      })
      .collect();

      return Ok(links);
}

//url lets us add relative urls to base urls
#[tokio::main] //designates main as an async function 
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let mut queue = VecDeque::new();
    let mut visited = HashSet::new(); // set of unique elements for tracking visited urls
    queue.push_back("https://books.toscrape.com/".to_string());
    
    while let Some(link) = queue.pop_front() {
        if visited.contains(&link) {
            continue;
        }
        visited.insert(link.clone()); // its optimal to mark as visited prior to for loop
        let new_links = collecturls(&link).await?;
        for new_link in new_links {
            if !visited.contains(&new_link) {
                println!("{}", new_link);
                queue.push_back(new_link);
            }
        }
    }

    Ok(()) //this line means we executed the code without any errors
}