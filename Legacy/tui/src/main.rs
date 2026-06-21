// ANCHOR: all
use std::{error::Error, io};

use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
        execute,
        terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    },
};

mod app;
mod ui;
use crate::{
    app::{App, CurrentScreen, CurrentlyEditing},
    ui::ui,
};

// the main.rs determines how to handle all logic happening within whatever screen is currently being displayed
// note to self you'll probably want to serialize your top-k queries with json for easy transport.

// ANCHOR: main_all
// ANCHOR: setup_boilerplate
fn main() -> Result<(), Box<dyn Error>> {
    // setup terminal
    enable_raw_mode()?;
    let mut stderr = io::stderr(); // This is a special case. Normally using stdout is fine
    execute!(stderr, EnterAlternateScreen, EnableMouseCapture)?;
    // ANCHOR_END: setup_boilerplate
    // ANCHOR: application_startup
    let backend = CrosstermBackend::new(stderr);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let mut app = App::new();
    let res = run_app(&mut terminal, &mut app);
    // ANCHOR_END: application_startup

    // ANCHOR: ending_boilerplate
    // w/o this the app will not close properly and a user will need to researt the terminal
    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    // ANCHOR_END: ending_boilerplate
    // ANCHOR: final_print
    // final thing outputted to terminal. might delete later
    // could i print out an ascci waifu?
    // if let Ok(do_print) = res {
    //     if do_print {
    //         app.print_json()?;
    //     }
    // } else if let Err(err) = res {
    //     println!("{err:?}");
    // }
    res?;

    Ok(())
}
// ANCHOR_END: final_print
// ANCHOR_END: main_all

// ANCHOR: run_app_all
// ANCHOR: run_method_signature
fn run_app<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> io::Result<bool>
where
    io::Error: From<B::Error>,
{
    // ANCHOR_END: run_method_signature
    // ANCHOR: ui_loop
    loop {
        terminal.draw(|f| ui(f, app))?;
        // ANCHOR_END: ui_loop

        // ANCHOR: event_poll
        // ANCHOR: main_screen
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Release {
                // Skip events that are not KeyEventKind::Press
                continue;
            }
            match app.current_screen {
                CurrentScreen::Main => match key.code {
                    // when e is pressed change current screen to editing
                    // sets currently editing to Some which should be a query
                    KeyCode::Char('e') => {
                        app.current_screen = CurrentScreen::Editing;
                        app.currently_editing = Some(CurrentlyEditing::Query);
                    }
                    //when q is pressed show the exiting screen
                    KeyCode::Char('q') => {
                        app.current_screen = CurrentScreen::Exiting;
                    }
                    _ => {}
                },
                // ANCHOR_END: main_screen
                // ANCHOR: exiting_screen
                // same as above just a yes our no to exit.
                CurrentScreen::Exiting => match key.code {
                    KeyCode::Char('y') => {
                        return Ok(true);
                    }
                    KeyCode::Char('n') | KeyCode::Char('q') => {
                        return Ok(false);
                    }
                    _ => {}
                },
                // ANCHOR_END: exiting_screen
                // ANCHOR: editing_enter
                //
                CurrentScreen::Editing if key.kind == KeyEventKind::Press => {
                    match key.code {
                        // what to do when we hit enter.
                        KeyCode::Enter => {
                            if let Some(editing) = &app.currently_editing {
                                match editing {
                                    CurrentlyEditing::Query => {
                                        if let Err(err) =
                                            app.get_search_results(app.query_input.clone())
                                        {
                                            app.search_results.clear();
                                            app.search_results.push(format!("Search error: {err}"));
                                        }

                                        app.record_search();

                                        app.current_screen = CurrentScreen::Main;
                                        app.currently_editing = None;
                                    }
                                }
                            }
                        }
                        // ANCHOR_END: editing_enter
                        // ANCHOR: backspace_editing
                        KeyCode::Backspace => {
                            if let Some(editing) = &app.currently_editing {
                                match editing {
                                    CurrentlyEditing::Query => {
                                        app.query_input.pop();
                                    }
                                }
                            }
                        }
                        // ANCHOR_END: backspace_editing
                        // ANCHOR: escape_editing
                        KeyCode::Esc => {
                            app.current_screen = CurrentScreen::Main;
                            app.currently_editing = None;
                        }
                        // ANCHOR_END: escape_editing
                        // ANCHOR: tab_editing
                        // KeyCode::Tab => {
                        //     app.toggle_editing();
                        // }
                        // ANCHOR_END: tab_editing
                        // ANCHOR: character_editing
                        KeyCode::Char(value) => {
                            if let Some(editing) = &app.currently_editing {
                                match editing {
                                    //interesting the string just a stack
                                    CurrentlyEditing::Query => {
                                        app.query_input.push(value);
                                    }
                                }
                            }
                        }
                        // ANCHOR_END: character_editing
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        // ANCHOR_END: event_poll
    }
}
// ANCHOR: run_app_all

// ANCHOR_END: all
