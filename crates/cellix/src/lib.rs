//! The Cellix application: state, views and the terminal loop.

mod agents;
mod app;
mod editor;
mod explorer;
mod find;
pub mod integrations;
mod menu;
mod neovim;
mod palette;
pub mod pane;
pub mod plugin;
mod rendered;
mod review;
mod search;
mod settings;
mod settings_page;
mod terminal;
mod theme;

pub use app::App;
pub use terminal::{Options, run};
