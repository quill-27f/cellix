# Progress

Newest entry last. Each entry says what changed, what is unverified, and what is next.
Feature status lives in `harness/features.json`.

## 2026-11-01: harness

Added the resource guard, task runner, cargo hook, feature ledger, CI and the
crates.io placeholder. Guard behaviour was proved by hand: it killed a memory hog
through a parent process, a process ignoring TERM, and a timed-out job, with no
leaked processes. Those proofs are not automated.

## 2026-11-01: pixel tiers

Pixel tiers work in Ghostty, tmux and Zellij, checked by screenshot on a static page.
Real-terminal checks found two bugs a fake terminal could not: a resize lost during font
loading, and Zellij falling to cells because the window size had no pixels yet.

Unverified: live scrolling, pane switching and detach, a real SSH link, other terminals.

## 2026-11-01: review, the first useful release

Ctrl+G opens side-by-side changes against HEAD. Hunks can be accepted or rejected with
the mouse, and any line can take a comment. `cellix --wait <file>` appends comments to
the file on exit, for `$VISUAL`. `cellix comments` reads them over the workspace socket.
Quit and close now ask before discarding unsaved edits, which they silently did before.

Accept only marks a hunk: it doesn't touch the file or the git index, and marks are lost
on exit. Reject refuses to run over unsaved edits.

## 2026-11-01: language features, search, settings

Added project search and replace, Markdown preview, settings with light and dark themes,
an LSP client (diagnostics, definition, completion, rename) and a Claude Code plugin
with a skill that reads comments back. The bridge mod moved to its own ledger entry.

Unverified: the LSP client has only run against a scripted server in the tests.
`real_rust_analyzer_finds_a_definition` covers a real server but is ignored, and has not
passed: this machine's rust-analyzer is a stub. Positions assume the server honours the
requested UTF-32 encoding, so a UTF-16-only server will misplace non-ASCII columns.
Replace is plain text with no preview, and skips files with unsaved edits.

## 2026-11-01: Vim mode and real tmux tests

Added a built-in Vim subset behind `"keymap": "vim"`. `harness/x mux` now runs the real
binary in tmux for typing, splits, resizing and window switching, on the cell tier.

Next: prove live scrolling and pixel tiers under tmux and Zellij, which `docs/DESIGN.md`
lists as the condition for pixels staying the default. Then the bridge mod, Neovim mode,
agent hosting, the wgpu rasterizer and DAP.
