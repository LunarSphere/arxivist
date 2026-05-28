//define enums and structs to represent what the user is currently interacting with
//basic logic related to the enums or structs

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
    pub search_results: HashSet<String>,
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
            search_results: HashSet::new(),
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
}
