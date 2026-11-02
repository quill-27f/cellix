//! The Cellix application: state, views and the terminal loop.

mod agents;
mod app;
mod dock;
mod editor;
mod explorer;
mod find;
pub mod hold;
pub mod integrations;
mod kitty;
mod menu;
mod neovim;
mod outline;
mod palette;
pub mod pane;
mod panels;
mod pdfview;
pub mod plugin;
mod rendered;
mod review;
mod search;
mod settings;
mod settings_page;
mod snippets;
mod spell;
mod terminal;
mod theme;
mod threads;
mod toolbar;

pub use app::App;
pub use terminal::{Options, run};
