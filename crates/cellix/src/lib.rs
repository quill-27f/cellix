//! The Cellix application: state, views and the terminal loop.

mod agent_task;
mod agents;
mod app;
mod brackets;
mod conflicts;
mod context;
mod dock;
mod editor;
mod editorconfig;
mod explorer;
mod field;
mod find;
mod fold;
mod folders;
mod graph;
mod guide;
pub mod hold;
mod inbox;
pub mod integrations;
mod kernel;
mod keys;
mod kitty;
mod linger;
mod menu;
mod neovim;
mod outline;
mod palette;
pub mod pane;
mod panels;
mod pdfview;
pub mod plugin;
mod recent;
mod rendered;
mod review;
mod rows;
mod search;
mod settings;
mod settings_page;
mod slow;
mod snippets;
mod source;
mod spell;
mod tasks;
mod terminal;
mod theme;
mod threads;
mod timeline;
mod toolbar;
mod transcript;
mod wheel;
mod workspaces;

pub use app::App;
pub use terminal::{Options, run};

/// "1 line" or "N lines", as everything that counts lines says it.
pub(crate) fn lines_word(count: usize) -> String {
    if count == 1 {
        "1 line".to_string()
    } else {
        format!("{count} lines")
    }
}
