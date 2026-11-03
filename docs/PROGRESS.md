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

Documents became the main surface. Markdown and LaTeX share one editable page, with typeset math
from pure Rust, a toolbar, a formula panel, table tools and images. A side pane runs agents and
shells on a real PTY, holds comment threads an agent answers with `cellix reply`, and shows PDFs.
`tectonic` builds were run for real through the binary: the PDF appears beside the page, a click
on it lands near the source line, and an error lands on its line.

Seven reviewers then used the real binary. They found what tests hadn't, and these are fixed:
Ask agent ran a comment's words in a shell, Ctrl+S could erase an edit made on disk, Ctrl+Q killed
running jobs, `cellix reply` reported success for a comment that didn't exist, agents weren't told
threads existed, and comments lost their place. The gate slipped once this round, when a commit
went through because a script read the exit of `tail`. `harness/x verify` now reads each ledger
test's result from the log, and a pre-commit hook refuses any tree it didn't stamp.

Tests changed because behaviour did: `agents_open_in_a_split_of_the_host`,
`ghostty_gets_a_new_window_for_an_agent` and `claude_starts_with_the_port_to_connect_to` went with
launching in host splits, and `comments_are_sent_to_the_agent_pane` now drives the side pane.

Unverified: real Claude Code, Codex or Gemini in the side pane, and a real agent answering with
`cellix reply`. Real Ghostty after the side pane and page work, since the screen was locked.
Over SSH. `sample` papers needing packages outside tectonic's cache.

Not done, from the reviews: Chinese and Japanese stay blank in cells (Korean works), so the font
fallback still isn't reaching them. Align spacing and big-operator limits are the math engine's.
tikzcd and commutative diagrams stay as source. Jobs don't outlive Cellix. No static Linux binary,
so a GPU box needs Rust. No CSV or notebook view. No tracked changes, spell check, outline or
build log panel. Codex and Gemini have `cellix plan` but no plan-held hook. The harness's headless
terminal clips the status band, which a real window doesn't.

Next: the user tests Claude Code, Codex and SSH.

## 2026-11-02: sixth round, documents in full, and a companion everywhere

The user's reports first. A click in an empty table cell had no text to land on, so the caret went to
the end of the row: empty cells now have a place of their own. The side pane had no way to close: it
has an × that stops nothing. Open in Browser left Cellix: the pane's terminal now reads the Kitty
graphics protocol, so `terminal-browser` runs there, proven with the real one. That work found
programs in the pane were told the wrong background, because the check used a palette number.

Documents gained Mermaid, Graphviz and SVG diagrams from pure Rust, TikZ and `tikzcd` built in the
background and proven with real `tectonic`, image sizing, dropping, pasting and a drag handle,
footnotes, callouts, highlights, tracked changes, spelling and grammar with Harper, an Outline, a
build Log, CSV tables, and notebooks that save back with their outputs.

The missing list from round five: Han characters drew blank in code because a monospace search
stopped at a font of blanks. Operators had their limits at the side, and `arg\\,min` printed a comma,
which was ours to fix. `\\sum\\limits` in text is still the engine's. Sessions now outlive Cellix
under a small host, proven by a job that kept writing through a quit and a rejoin, and stopped on Stop
and Quit. A `release` workflow built a static Linux binary on CI. The headless terminal now shows
images over the window's padding as Ghostty does, which was what cut off the status band.

The user then asked for guidance and a companion approach. An empty window lists clickable actions and
names the next step for what's running, the agents panel says how it works, and Getting Started is a
real page. A selection offers Comment and Ask Agent in every file type, and pictures, diagrams,
formulas, PDFs and images take comments too.

Tests changed because behaviour did: `cli_reads_comments_over_socket` waits for the side pane, since
comments on code are now written there. `a_comment_sits_in_its_card` closes the pane first, since its
text now shows there too. Several rail tests moved with the new Outline icon. Two table tests looked
for one letter a temporary folder's name could hold, so they failed at random, and now use words.

Unverified: real Claude Code, Codex or Gemini in the pane, in the fifth round's list and still. A real
Ghostty window for any of this round's work, including the browser and Keep Running, which ran
through the headless terminal. SSH. Linux with the static binary, past `--version` on CI.

Not done: Codex and Gemini still have no plan-held hook, since neither could be tested here and it
would be a guess. Web export leaves `tikz` blocks as source. A notebook's comment line counts page
lines, not JSON ones. Cells don't run. Jobs held for a project are only found from that same folder.

Next: the user plays with `samples/` and tests Claude Code, Codex and SSH.

Round six, continued while the user tests. The design review's smaller items went in: the end of a
proof closes its last line, captions stand in the middle of the page, labels clear their headings,
text scrolling under the toolbar has a line to go under, and the agents list reads as a tab.


Round seven, from the user's notes while testing. Notebooks run in a real Jupyter kernel with their
own bar and Jupyter's keys. The minimap shows pictures. Ask Agent no longer sticks when no agent is in
the pane: the comment waits for the next prompt or the next agent started. The pointer is set with
OSC 22 and put back. Tables take Excel's keys. Tabs in the pane close. The status bar's error count
opens a Problems tab.

The rail's Git icon now opens Source Control in the sidebar, as VS Code's does: a message box, Commit,
Push or Pull with counts, Staged Changes and Changes, and hover marks to stage, unstage and discard.
A click on a file shows its diff beside the panel. Ctrl+G still opens the full Review, which keeps
its own list unless the panel is showing. `activity_bar_opens_review_by_mouse` moved with the icon,
and the tour's review scenes now use Ctrl+G, with two new scenes for the panel.

Unverified: push and pull against a real server with credentials, only a local bare repository was
tried. Partial staging of lines isn't in the panel; hunks can still be staged from Review.

Later in round seven. The right pane got a set of icons at the top right, like the Claude desktop
app's: Comments, Terminal, Review, Browser, Agents. They replace the Agents icon on the rail, and
the agent-needs-you dot moved with it. Ctrl+J brings a terminal up and puts it away, except in an
agent, where it stays a new line. Middle click closes pane tabs. Agents is now a real tab. Its list
was only filled when the pane opened through its toggle, so Ask Agent dead-ended on an empty list.

In a window too narrow for side by side, the pane stands under the file. A short window gives it
everything, and the edge can't be dragged there yet.

The browser was broken only under Ghostty, where it draws in tiles: rectangles at pixel offsets in
z order. The terminal emulator holds text back during a synchronized update, cursor moves included,
so every tile landed in one spot. It's flushed before a picture now, and offsets, layers and
single placements are honoured. `stored_images_show_on_request_and_can_be_deleted` changed: a
lowercase delete keeps the image, as the protocol says, so the test now uses the capital.

Not reproduced: lag typing a comment after Ask Agent. Frames took under 25 ms in a debug build, with
one frame per key and less data than typing on the page. Unverified: a real Ghostty for the browser,
which ran headless with Ghostty's environment set. This commit holds several features, because the
pane changes touch the same code and verifying each alone wasn't worth the load on this machine.

The pane now holds only what was opened. Comments was a fixed tab and the list of agents stood in
when nothing else was left, so a terminal opened beside Comments and closing it showed Agents. Now
each is a tab with a ×, closing one goes to what's left, and closing the last closes the pane. The
dot on the Comments icon says there are comments when they have no tab. Going through the pane's
own × still only hides it, and what runs there keeps running.

The design review's findings on the icons went in. They now stand in one place whatever the pane
does, on every surface, three columns in from the corner so the pane's × has a slot.
`a_narrow_pane_leaves_the_icons_at_the_end_of_the_files_tabs` became
`the_icons_stay_put_whatever_the_pane_does`, since the behaviour it pinned was the defect. The rail
lights only what's showing, and Review lights Source Control. Source Control rows gained file icons
and a folder set after the name as drawn. A notebook's dividers come from its own groups: they were
being read from the page bar's. Tabs in a narrow pane share the row, so long names are cut short
there. The tour gained scenes for hover, staged files and the diff beside the panel, and 13-review
was a tour timing problem, not a repaint bug: the shot came before the first key landed.

CI failed once on `with_nothing_staged_commit_takes_everything`. It was a real race: the panel's
background `git status` takes the index lock, so a commit at that moment was refused. Every read now
sets `GIT_OPTIONAL_LOCKS=0`, which also keeps Cellix out of the way of an agent's own commits, and a
read started before a change is dropped, so it can't put the old state back.

Finishing the open items. Plans are now held for Codex and Gemini too. Gemini's goes through
`BeforeTool` on `exit_plan_mode`, and Gemini still asks for its own approval afterwards. Codex has no
plan tool, so its Stop hook holds a `<proposed_plan>` block: sending back starts another turn, and
approving leaves implementing it for the user to ask for. Both were tested with hook input built
from the docs, not a real run, because Codex's hooks need the user's `/hooks` trust and Gemini isn't
installed. If Codex strips the tags from `last_assistant_message`, its hook just never fires.

A stacked pane's line can be dragged. In a narrow pane the tab in front keeps its name, and the
others show theirs on hover. Pane tabs share the file tabs' text and close button, and line up with
them when stacked.

The comment lag still can't be reproduced. The release build answered in 3 to 10 ms on the page and
about 5 ms in a comment, short or 600 characters long. So `~/.cache/cellix/slow-frames.log` now notes
any frame over 50 ms with where the time went, timings only, started afresh each run.

From testing. A CSV had no way back from its source, because that view had no toolbar: a button at
the end of the path row now goes between page and source, and the source's right-click menu says the
same. A click on a lit rail icon closes the left side, as in VS Code. Ctrl+B does too, unless words
are picked on a page, where it stays Bold. `ctrl_b_is_bold_on_a_page` became
`ctrl_b_is_bold_on_words_picked_on_a_page` for that, and the Outline test's second press closes now.

Scrolling is now a glide, as `docs/DESIGN.md` describes, in place of four fixed steps that blocked the
main loop. The old scroll tests were rewritten for it: `regions_scroll_by_pixels` and
`a_region_clips_at_its_edges` assert the same things frame by frame, and new ones cover the ease, a
notch landing mid-glide and turning round. Checked in the headless terminal, including mid-scroll on
a typeset page, about 2 MB/s there. Unverified: how it feels in a real Ghostty, which is the point of
it, and only editors glide. Lists, Review and the pane still move by lines.

Getting ready to go public. The history holds no secrets, personal paths or personal email, and all
726 dependencies have permissive or file-level licences. The plugins moved into
`crates/cellix/integrations`, since `cargo publish` packs only what's inside a crate. The release
workflow builds four targets and writes a Homebrew formula from their checksums, and publishes to the
tap and crates.io only where their tokens are set. The README leads with what Cellix is to an agent.


The lag and the scrolling had a cause my tests couldn't see. `slow-frames.log` on the user's machine
showed the main thread stalling 60 to 170 ms in event handling about every two seconds, with drawing
near zero: 158 of them in five minutes. That was the periodic look at the tree and git, which ran
`git show` and a fresh diff for every open file on the main thread. Small test folders hid it. It's on
a thread now, and only touches an editor when something really changed. `explorer_shows_git_status`
and `new_files_appear_in_the_explorer_by_themselves` wait for it, since it no longer finishes in the
tick.

Scrolling again, from the log and from reading how Claude Code does it. Its fullscreen view has no
animation: a trackpad's flood moves a line per event, and a wheel's speed ramps and fades. So the
first event no longer jumps three lines, and the ease went from 40 ms to 14, since a longer one
trails the hand and reads as lag. Tiles are now cut into bands, so a scrolled row keeps its tile and
settling draws nothing new, which was up to 195 ms on the user's machine. Queued input can't hold up
a frame for more than 4 ms, and a trackpad wandering sideways no longer ends a glide. The rail's
icons and the top-right bar were also made smaller and airier, as asked.
`the_wheel_reports_what_it_moved` changed with the first event. Tests that placed the icons moved
with the new margin. How it feels in a real Ghostty is still for the user to judge.

The README now leads with Cellix inside an agent, and with writing documents alongside it. The editor
on its own comes after. `cellix setup claude`, `codex` and `gemini` make that one line each, doing
what the Install buttons do. Its screenshots still show Cellix on its own: a shot from inside Claude
Code needs a real terminal window, which wasn't taken while the user was testing.

Public, with the user's go-ahead. v0.1.0 is released for macOS and Linux on both architectures, and
`quill-27f/homebrew-tap` holds a formula checked against the release's checksums. crates.io still has
the 0.0.1 placeholder until the user publishes with their own token, so the README installs from git
for now. The README still lacks a screenshot from inside Claude Code: a scripted Ghostty window never
opened here, and the user is sending one.

The README opens with a real screenshot of Cellix inside Claude Code: a Ghostty window started with
`open -na`, Claude Code fed its keys through `harness/live/pty_inject.py`, the window captured with
`screencapture -l`, in a scratch folder the user trusted. Ghostty's own binary started from a script
never opened a window. `/cellix` takes no file, so `/cellix-open <file>` is the one for a document. A
second Claude Code in the same folder waits on "Starting Cellix…" while the first still holds its
Cellix, which is worth fixing. The Getting Started guide and `samples/writing.md` still put the
Agents icon on the left, and now say the top right.


The rail is four columns, down from five, with its icons at about half their old size and the pill
drawn to fit. `a_page_uses_the_whole_width` now allows a line to stop a word and the minimap's gap
short of the edge: one column less moved where the last word broke, not how wide the page is.

Round eight, from four persona reviews. The built-in Vim mode corrupted files: `ciw` wasn't known, so
the letters after it ran as commands and `O` opened a line that took the rest as text. The `vim`
keymap now runs the real Neovim when it's installed. Without it, the built-in mode knows `iw` and
`aw`, `w` and `e` as Vim has them, `f`/`t`/`F`/`T`, `V`, `D`, `C`, `J`, `r`, `/` and the `:` commands
that save, close and go to a line, and anything else is swallowed rather than typed. Its tests run
the built-in mode whether or not this machine has Neovim.

Export. Front matter becomes the title, author and date in both the HTML page and the PDF, where it
printed as a heading of `key: value` lines. `==` inside inline code stays code instead of a
highlight. A document with pending suggestions asks whether the PDF shows them accepted or as they
stand. The PDF builds from a hidden `.name.export.tex` and its log and aux files go, so an existing
`name.tex` no longer stops an export. Unverified: the latexmk and pdflatex leftovers, on a machine
without tectonic.
