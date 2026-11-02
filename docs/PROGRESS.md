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

## 2026-11-01: first real use, and a redesign

Using it in a tmux window showed what no test had. The caret was hashed into every row's
tile, so each keystroke resent the whole screen: about 570 KB per key, now about 140 KB
in a palette and less in the editor. `crates/cellix/examples/frames` measures this.

Also fixed: Mac text shortcuts, middle-click to close a tab, hidden and ignored files in the
explorer, headings painting over the status bar, and the hover pill hiding its icon. On a
known terminal background all chrome now sits on that colour, because Ghostty's window
padding showed as a seam. The test for that changed on purpose: panels no longer keep a
fixed distance from the editor.

Unverified live: all of the above, including Cmd+Z outside tmux. The wheel moves one line at
a time under tmux and three on the direct tier.

A redesign is on a Claude Design canvas, waiting for the user's review. Next, once it's
agreed: build it, then run Cellix as a pane inside Claude Code through a mod. The mod API
has what that needs, panes showing images plus pointer and key events, but none of it has
been tried. An image named by file can't cross SSH, so remote sessions would have to send
PNGs.

## 2026-11-01: the editor, the redesign and the Claude Code pane

Added ten grammars, a Terraform query, and line-based highlighting for `.env`, Dockerfile,
INI and ignore files. Suggestions work with no server. Measuring found a full reparse on
each key in large files: a keystroke in a 5,800 line file took 200 to 600ms to draw, under
heavy load, and now takes about 10ms in all, with colours from a background thread.

Added margin marks for changes, a whole-file minimap and bracket pairs. Built the approved
canvas. The review view keeps its side-by-side layout on wide screens, rather than the
canvas's stacked cards.

Cellix now runs as a pane inside Claude Code. Checked live in Ghostty against 2.1.309 with
`harness/live/pty_inject.py`: a click places the cursor, typing reaches the pane, the wheel
scrolls, and a proposed edit opens the pane by itself. One live run was stopped by the GPU
guard, whose measure covers the whole machine. An idle pane was resending its image twice a
second, which is fixed, and later runs stayed in budget. No cause was proven.

Unverified: the pane over a real SSH link, under tmux, and with a plan arriving. Other
terminals. `terraform-ls` and the other servers, since only rust-analyzer is on this
machine. Plain `claude` without `--ide` won't send edits to the pane. The pane uses the
dark theme, because it can't ask a terminal Claude Code owns for its background.

The real rust-analyzer suite still has the open failure from the last entry.

Next: the user tries it.

## 2026-11-01: second round of real use

The user found the build didn't match the approved canvas, and more. Fixed the space bar in
the pane, whose key name was dropped. The minimap bounced because the smooth scroll layer
took it in, and its marker was nearly invisible. Pages are now full width, by the user's
choice over the canvas, with paragraphs flowing across source lines. Questions are dialogs
with buttons. Review is stacked cards as the canvas shows. Side-by-side is gone.

The pane's bands above and below are Claude Code's own frame, proven by painting our box
magenta and seeing none of it outside the image. The dock is capped at 45% of the window.
Neither can be changed from a mod.

Unverified: the smooth scroll fix in a real window, because the screen locked before it
could be captured. It's covered by tests of what scrolls, not by eye. Over SSH as before.

Next: the user checks scrolling and the canvas.

## 2026-11-01: third round of real use

The tile key left out the row, so the empty rows around a tall heading shared a tile and
its lower half showed blank. That only happened past the minimap, which is why a small
scroll seemed to fix it. Snapshots draw rows one at a time and could never show this, so
`harness/live/term.py` is now a headless terminal for the real binary. It keeps Kitty
graphics, takes real keys and mouse, and writes PNGs. `harness/x tour` walks every surface
through it, and the `ui-expert` agent reviews those pictures.

New: right-click menus on pages, code, tabs and the explorer. Double and triple click.
A branch switcher from the sidebar or the status bar. An Agents panel with something to do
in it. A real Settings page. `cellix setup --install`. Compare-with no longer waits for
git on the main thread: a click went from about 600 ms to under 100 ms on a debug build.

A new test caught a real bug: typing with several carets on a page collapsed them after
the first letter, which was the likely cause of the Ctrl+D report.

`tables_edit_in_their_cells` was wrong: it looked for "a " anywhere on screen, and failed
when the random temp folder in the sidebar ended in that letter. It now looks for a word
that can't appear by chance. `plan_review_returns_comments` now goes through the menu,
since a right click no longer jumps straight to a comment.

The features here share one commit, against the rule, because they all touch the same
paths through `app.rs` and splitting them would have meant commits that don't build.

An independent review measured every surface and found the spacing complaint had one
cause: lines and pills sat on row boundaries, the only 8 pixels of air a row has, where the
boards have 25. Separators now take a blank row, and the palette has a real header. A
second pass checked the fixes and reviewed the two Settings pages the first couldn't see.

Unverified: the Ghostty window for an agent, which would take focus, is covered by a test
of its command only. Cmd+D after freeing it, since that needs Ghostty. Computer use on
Ghostty was declined, so nothing here ran in a real window: `harness/live/term.py` stands
in for one. Codex has no plugin, because it isn't on this machine to build against.

Open from the review: matched letters and file icons in the palette, list pills in review,
table row padding, hover states in the tour. Menus show Ctrl because Ghostty keeps Cmd+C.

Next: the user tries it, and clicks Install under Settings, Integrations.

## 2026-11-02: fourth round, frame, languages and agents

Ghostty's padding can't be drawn in, which is why the app never filled the window. Tiles now
sit on real cell colours at the edges, so `window-padding-color = extend` can carry them out,
and Settings can turn that on. Not seen in a real window yet: the screen locked first.

Added Go to References and Format Document, proven with a real rust-analyzer. Thirteen more
grammars and a table of 26 language servers, each installable from Settings. Plugins for
Codex and Gemini CLI. Hover has one grammar now, and the Agents panel leads with what's
running, since Cellix comes to the agent.

Real SSH works: the headless terminal drove Cellix on a Linux machine in OrbStack over `ssh`,
and an edit landed in the file there. A key took 19 ms.

A test mistake to learn from: the server install used a login shell, whose profile replaced
the PATH holding a fake `npm`, so the real one started and left two dangling links in
Homebrew's folder. They were removed. The install now uses the PATH Cellix was given.

Unverified: real Ghostty and Terminal.app, including hover with a real pointer, since the
screen was locked. A real Codex run, because its hooks need trusting with `/hooks` and
bypassing that isn't ours to do: only its documented input was tested. A real Gemini run,
which needs a sign-in. Antigravity CLI, which Google says replaces Gemini CLI, isn't covered.

Next: the user checks the window fill, and trusts the Codex hooks.

## 2026-11-02: fifth round, writing and side-by-side work

Ghostty doesn't clip images, so a terminal app can reach its padding after all, which the
last entry said it couldn't. A real window proved it, and the app now fills to the right and
bottom edges with no config. Match edge is gone, with its test
`the_window_is_filled_once_and_keeps_the_config`, because the feature no longer exists. Its
ledger entry now points at `tiles_only_leave_the_text_area_on_ghostty`.
