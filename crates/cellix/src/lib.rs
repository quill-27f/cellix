//! The Cellix application: state, views and the terminal loop.

mod agents;
mod app;
mod editor;
mod explorer;
mod find;
mod neovim;
mod palette;
mod plugin;
mod rendered;
mod review;
mod search;
mod settings;
mod terminal;
mod theme;

pub use app::App;
pub use terminal::{Options, run};
