//! The guide a new person can open: a real page, so reading it also shows what a page does.

use std::io;
use std::path::{Path, PathBuf};

const GUIDE: &str = r#"# Getting started

Cellix is an editor for working beside agents, and for writing. Everything here works with the mouse, and every key is listed in **Settings**.

> [!TIP]
> This page is a real document. Click anywhere and type: nothing here is saved anywhere that matters.

## Work beside an agent

1. Click the **Agents** icon on the left, the fourth one down.
2. Pick an agent. It starts on the right, in this project, and it's the real program.
3. Click a file in the sidebar. It opens beside the agent, so you see what it changes as it changes it.
4. Press `Ctrl+`` ` to move the keyboard between the file and the agent.
5. Press the **×** at the top right of the pane to hide it. What runs there keeps running.

## Talk about the work

- Select text in any file, then press the comment button or `Alt+C`.
- Press **Ask Agent** on a comment, and the answer lands under it.
- Open **Review** from the left to see every change, accept what's right and undo what isn't.

## Write

The bar above a document holds everything: formats, a table, a formula, a picture, a diagram, comments and track changes.

- Right-click a table to add rows and columns. Click any cell to type in it.
- Right-click a picture to resize, replace or delete it. Drop a file on the page to add one.
- Click a formula or a diagram to edit its source, with the result in view.

Math sets as you write it, here as $e^{i\pi} + 1 = 0$:

$$
\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}
$$

And a diagram is a few lines of text:

```mermaid
flowchart LR
  You[You ask] --> Agent[Agent works]
  Agent --> Review[You review]
  Review --> You
```

## Papers

A `.tex` file opens as a page too. The play button at the top right builds the PDF beside it, and clicking the PDF goes to that place in the source.

## Find your way

- `Ctrl+P` opens a file, and `Ctrl+Shift+P` lists every command.
- The **Outline** icon on the left shows a document's headings.
- Open this guide again from the commands, as **Getting Started**.
"#;

/// Writes the guide somewhere it can be opened, and says where.
pub fn write(folder: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(folder)?;
    let path = folder.join("Getting Started.md");
    std::fs::write(&path, GUIDE)?;
    Ok(path)
}
