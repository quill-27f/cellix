//! The Cellix application: state, views and the terminal loop.

mod app;
mod editor;
mod explorer;
mod terminal;
mod theme;

pub use app::App;
pub use terminal::run;
