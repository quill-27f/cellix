//! Editing model: text buffers, selections, undo and the workspace tree.

mod buffer;
mod selection;
mod workspace;

pub use buffer::Buffer;
pub use selection::{Motion, Selection, Selections};
pub use workspace::{Entry, Workspace};
