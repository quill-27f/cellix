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

## 2026-11-01: first real use

Feedback from using the binary drove this pass.

Scrolling on the pixel tier left stale rows on screen, because each tile's old placement
stayed when a new image took its row. Fixed, with a test that models the terminal's
placement rules and failed before the fix. Still needs confirming in a live terminal.

The look now follows VS Code: Dark+ and Light+ colours, activity bar, breadcrumbs, status
bar with branch and language, current-line highlight, indent guides and a clickable
scrollbar. Tabs close with `×`. Markdown opens with source and live preview side by side.

rust-analyzer now runs in the ignored tests: definition, rename and completion pass, and
found real bugs the scripted server could not, including a rename applied twice.

An earlier version of this entry said compiler errors never appear. That was wrong: they
appear after a save. The real gap was narrower and worse: when an agent rewrote an open
file, no save reached the server, so `cargo check` never ran. Reloads now count as saves.
Run these with `harness/guard.sh -- cargo test -p cellix --lib real_rust_analyzer -- --ignored`.

Unverified: live scrolling in a real terminal after the overlap fix, other language
servers, and the look on the cell tier in terminals with weak Unicode fonts, where the
activity bar icons may be small or missing.

## 2026-11-01: second round of real use

Markdown is now one editable rendered page, replacing the side-by-side preview after
reading how Xcode's rendered editor behaves. Search is a live sidebar panel. Activity
icons are real shapes spanning three by two cells. Ctrl+Shift+P and Cmd keys work through
the Kitty keyboard protocol. Narrow panes get a layout of their own.

Unverified: all of this in a live terminal, Ctrl+Shift+P in tmux and Zellij, and whether
Xcode hides inline markers such as `**`. Ctrl+F opens project search, because find in the
current file doesn't exist yet. Tables wider than the page wrap rather than scroll, and
code blocks on the page have no syntax colours.

## 2026-11-01: paint layer

Feedback was that it still read as an ASCII drawing, so the pixel tiers now paint the
interface: Inter for chrome and JetBrains Mono for code, both embedded so SSH hosts with no
fonts get the same look. Vector file and folder icons, rounded inputs, buttons and popups
with shadows, hairlines, hover states and a thin caret. Prompts float at the top like VS
Code's quick input. Scrolling was confirmed fine in a live terminal.

`Open in Browser` and Ctrl+click use terminal-browser in a split, falling back to the
system browser.

Next: per-region tiles for smooth pixel scrolling, then a proportional Markdown page.

Checked by screenshot on a static page in Ghostty, tmux and Zellij, where the paint layer
looks the same in all three. The block-art icons on the cell tier are crude.

Unverified: terminal-browser opening a real split (the tests use a stand-in command), a
real SSH session, and paint-heavy rows under tmux, where each tile costs more to send.

## 2026-11-01: end-to-end interface audit

Compared against Xcode and VS Code from screenshots. Text is now measured, so carets sit
in the right place and stay inside text boxes. The Markdown page uses real typography, and
the mouse reports pixels where the terminal can. Theme is auto, dark or light, from OSC 11.
Claude Code and Codex start in a split of tmux or Zellij, replacing the plan to host them.

Unverified: all of this in a live terminal, pixel mouse under Ghostty, OSC 11 through tmux
and Zellij, and starting a real agent. Outside tmux and Zellij there's no split to open an
agent in, and Cellix says so. Xcode's formatting toolbar isn't built.

One run of the real rust-analyzer tests failed after 110 seconds, then two runs passed in
20 seconds each. The failing test wasn't captured, so treat that suite as possibly flaky.

## 2026-11-01: the eight companion features, and all 68 features

Built everything agreed for the companion role: review by agent turn with undo, real hunk
staging and commit, smarter diffs, comments sent to the agent, plan approval, agent and
worktree status, test results in review, and the Claude Code IDE connection. Also find in
file, the Markdown toolbar, smooth scrolling, the status mod and a Neovim keymap.

A design pass redrew the icons as one set, moved surfaces to VS Code's Modern themes, and
made `auto` match the terminal's real background.

Proven live against Claude Code 2.1.309, driven in tmux: checkpoints labelled by prompt,
test results in review, a plan approved and sent back, proposed edits opening in Cellix and
accepted there, answering in Claude's prompt closing the diff, a selection reaching
Claude's prompt, `Open Claude Code` from the palette connecting by itself, the mod's status
entry and `/cellix-comments`.

Live runs found what tests couldn't. `PreToolUse` leaves Claude's plan dialog up, and an
allow needs `updatedInput`. A plan's tab title showed the session id.

Smooth scrolling was checked in real Ghostty by freezing output mid-animation with
`harness/live/scroll.sh`. The frame sits between rows, clips under the breadcrumb, and
leaves the sidebar still, and the settled page has no leftovers. How it feels by hand, and
its cost over a slow link, are unknown. Over SSH it uses three frames instead of six.

Neovim mode was checked in tmux on the cell tier with a real config: `dd`, `ciw` and `:w`.
Plugins that open floating windows or depend on a UI won't show, since only the buffer,
cursor and mode come back.

Open problem: `real_rust_analyzer_reports_compiler_errors` passed three of three alone, but
failed in a full run after the other real-server tests, on a machine with OrbStack using
eight cores. A save during startup was being lost, which is fixed: files get their check
when rust-analyzer reports it has loaded. The remaining failure isn't understood. It may be
test order, but that isn't proven. This suite isn't in `harness/x verify`.

Unverified: a real SSH link, which is the next step. Codex beyond starting it. Zellij's
Open Claude Code path, which sets the same environment but hasn't been run. Sending
comments straight to a pane only works in tmux, and Zellij waits for the next prompt. The
mod needs `cellix` on the path, which `Open Claude Code` provides.

`cargo publish` will fail until `integrations/claude-code` moves inside the `cellix` crate,
because `plugin.rs` embeds it from outside.

Next: the user tests the full set over SSH.
