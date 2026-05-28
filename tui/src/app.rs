//define enums and structs to represent what the user is currently interacting with
//basic logic related to the enums or structs
//
use anyhow::Result;
use queryengine::*;
use rusqlite::Connection;
use std::collections::HashSet;
//what the user is currently viewing
pub enum CurrentScreen {
    Main,
    Editing,
    Exiting,
}

//what field the user is currently inputing
pub enum CurrentlyEditing {
    Query,
}

//track the full application state
// tracsk data being passed around
pub struct App {
    pub query_input: String, // the current query.
    pub search_history: HashSet<String>,
    pub search_results: Vec<String>,
    pub current_screen: CurrentScreen, // the current screen the user is looking at, and will later determine what is rendered.
    pub currently_editing: Option<CurrentlyEditing>, // what box are we editing
}
//
impl App {
    //init the app state
    pub fn new() -> App {
        App {
            query_input: String::new(),
            search_history: HashSet::new(),
            search_results: Vec::new(),
            current_screen: CurrentScreen::Main,
            currently_editing: None,
        }
    }
    pub fn record_search(&mut self) {
        self.search_history.insert(self.query_input.clone());
        self.query_input = String::new();
        self.currently_editing = None;
    }

    //print search result?
    pub fn get_search_results(&mut self, query: String) -> Result<()> {
        // take search query as command line args and normalize it
        // connect to db
        self.search_results.clear();
        let pool = Connection::open("../data/crawler.db")?;
        // search for pages (db, query, k-ranks)
        let results = search_db(&pool, &query, 10)?;
        for (index, result) in results.iter().enumerate() {
            let mut result_str = String::new();

            result_str.push_str(&format!(
                "{}. {}\n    URL: {}\n    Score: {:.6}\n",
                index + 1,
                result.title.as_deref().unwrap_or("Untitled"),
                result.url,
                result.score,
            ));

            if let Some(snippet) = &result.snippet {
                result_str.push_str(&format!("   {}\n", snippet));
            }

            result_str.push('\n');

            self.search_results.push(result_str);
        }
        Ok(())
    }
}
