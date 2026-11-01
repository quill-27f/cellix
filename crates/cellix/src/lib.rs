//! The Cellix application: state, views and the terminal loop.

mod app;
mod editor;
mod explorer;
mod markdown;
mod palette;
mod review;
mod settings;
mod terminal;
mod theme;

pub use app::App;
pub use terminal::{Options, run};
