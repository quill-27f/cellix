//! Editing model: text buffers, selections, undo and the workspace tree.

mod buffer;
pub mod pattern;
mod selection;
mod workspace;

pub use buffer::Buffer;
pub use selection::{Motion, Selection, Selections};
pub use workspace::{Entry, Match, Workspace};
