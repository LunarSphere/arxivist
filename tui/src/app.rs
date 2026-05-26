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
    pub query_input: String,                         // the current query.
    pub current_screen: CurrentScreen, // the current screen the user is looking at, and will later determine what is rendered.
    pub currently_editing: Option<CurrentlyEditing>, // the optional state containing which of the key or value pair the user is editing. It is an option, because when the user is not directly editing a key-value pair, this will be set to `None`.
}
//
impl App {
    //init the app state
    pub fn new() -> App {
        App {
            query_input: String::new(),
            search_history: HashSet::new(),
            current_screen: CurrentScreen::Main,
            currently_editing: None,
        }
    }
    pub fn record_search(&mut self) {
        self.search_history.insert(self.query_input.clone);
        self.query_input = String::new();
        self.currently_editing = None;
    }

    //print search result?
}
