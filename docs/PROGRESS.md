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
