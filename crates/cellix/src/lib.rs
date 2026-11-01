//! The Cellix application: state, views and the terminal loop.

mod app;
mod editor;
mod explorer;
mod palette;
mod rendered;
mod review;
mod search;
mod settings;
mod terminal;
mod theme;

pub use app::App;
pub use terminal::{Options, run};
