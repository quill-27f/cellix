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

LaTeX. Typing inside `\cite{` or `\ref{` (and `\citep`, `\eqref`, `\cref` and the rest) offers the
bibliography's keys or the paper's labels, and Enter or Tab puts the whole key in. A brace still
being typed shows on the page, where it vanished until closed. A reference to no label and a
citation the bibliography lacks show in the colour of failure, as LaTeX prints `??` and `[?]`.
latexmk and pdflatex now write SyncTeX, and pdflatex runs BibTeX and again until references settle.
Unverified: the pdflatex passes on a machine with only pdflatex.

Claude Code mod. `/cellix <file>` opens the file, where the argument was ignored. `/cellix-send`
and the pane's Send to Agent start a turn, and the prompt hook adds the comments to it, where they
waited for the next prompt. Claude Code refuses a prompt from inside a command, which would wait on
the turn the command holds, so it starts just after. A second session in a folder that already has
a Cellix says so, where it said "Starting Cellix…" for good; one Cellix per folder stays the rule,
because hooks reach a folder's Cellix by its socket. The mod has its own tests,
`hooks/register.test.tsx`, which `claude plugin test` runs in Claude Code's kit, and the Rust suite
runs them where Claude Code is installed.

Comments and agent edits. A thread follows its quote when the text is rewrapped across lines or
has a word or two changed, where it stayed on the old line. The Comment and Ask Agent bar stands
above the first selected line, where a selection dragged downwards had it over its own words, and
several cursors get no bar. With Track Changes on, a file an agent rewrites arrives as suggestions,
word by word, in CriticMarkup or LaTeX's changes package, to accept or reject before saving. The
skill tells agents so, and how to write a suggestion themselves. Unverified: a second agent edit
before the first is settled meets the usual "changed on disk" conflict.

Plan versions. Each plan an agent sends again keeps the one before it, `Plan v1.md` and on, beside
`Plan.md`, so comments stay on `Plan.md` and follow their text into the new version. The status
says which version is waiting, and Compare Plan Versions opens what changed as suggestions on a
page of its own while the plan waits.

Word count. The status counted every run of non-spaces, so `#`, `-`, front matter, code blocks,
link targets and a paper's preamble, commands, keys and math all counted. It now counts prose:
Markdown without its markup, code, comments and link targets, and a LaTeX body without comments,
commands, `\label` and `\cite` keys or math, with pending suggestions taken as accepted. A
selection shows as "12 of 340 words", and the count stays while tracking changes. Counted once a
revision. Unverified: a paper split over `\input` files counts only the file in front.

Search. Find in a file and Search in the project share one matcher, with VS Code's three switches:
Match Case, Match Whole Word and Use Regular Expression, as buttons and as Alt+C, Alt+W and Alt+R.
Find gains a replace row, on Ctrl+R or its ⇄ button (Ctrl+H is Backspace in too many terminals):
Enter replaces the match in view and moves on, Alt+Enter or All replaces every one as one edit,
and a regular expression's `$1` takes its group. Project search lists every match on a line, not
the first, and Replace All uses the switches. Ctrl+\ splits the editor where the terminal sends it
as the old 0x1c byte, which arrived as Ctrl+4. The tour has shots of both.

The plan banner names the agent whose plan is waiting, from its hook's event, where it always said
Claude Code.

LaTeX in the IDE. The file tree leaves out what a build writes beside a paper (`.aux`, `.log`,
`.synctex.gz`, `.fls` and the rest) when the paper is there, and keeps a `.log` with no paper. A
paper's right-click menu names LaTeX's own things, Typewriter, Paragraph, Section and Verbatim, and
offers Insert Citation, Build PDF and Show in PDF, where it had Markdown's menu.

Find and Search polish, from the ui-expert review of the tour. The switches sit inside the search
field's end, off with no chip, hovered with a quiet one and on with the selection colour and a
focus edge. Replace and Replace All are buttons straight after a replace field as wide as the find
field, and the ⇄ toggle stays lit while the replace row is open. Every input field is inset a few
pixels in its row, so stacked fields keep their air, and the Search panel's replace field sits
straight under its search field. The Find bar's tooltips sit under their button, clear of the bar.
A paper's menu puts Insert in its own group. Not done from the review: a slimmer Find bar, search
snippets that keep context with an ellipsis, and one colour for matches in the editor and the panel.

Rail and Find bar, rebuilt from the Claude Design canvas (https://claude.ai/artifact/LsovugNAutxD4dna8ZGGSV),
options C and A, which the user picked. The rail is three columns with Lucide-style line icons
(files, search, a tree of headings, a branch, two sliders for settings) one terminal row apart and
level with the sidebar's header, the one showing on a small chip, resting icons dimmer, the change
count on the chip's corner ringed in the rail's colour, and a hairline against the sidebar. The cell
tier shows one glyph per icon. The Find bar is a raised panel just around its rows: a chevron that
shows Replace, the switches inside the field, the count, then arrow and close icons, and Replace
and Replace All as icon buttons with their keys in tooltips. Four rail tests clicked fixed rows or
counted a 3 by 2 icon; they now follow `rail_row` and the 2 by 1 icon.

Browser install. On Linux under tmux, with no browser that would open, Cellix said to set
`browser` in the settings file. Now, when terminal-browser isn't on the path, the corner card that
offers language servers offers it, with the same Install, Not now and Never; Install runs its own
installer (`curl -fsSL https://terminal-browser.sh/install | bash`, from its releases page) and opens
the browser when it's done. Settings › Integrations has a Browser card with Install too. Unverified:
the installer on a real Linux machine.

UI pass from the whole-app style review. One icon set: files and folders are Lucide outlines,
chevrons, close, arrows and replace share the rail's line weight, and the rail's icons are sized
to look as large as the top right strip's (they had looked smaller). The strip's active chip is the
rail's chip, and its resting icons the rail's colour. The tab in front has an accent along its top.
With paint, a menu's groups part on a hairline instead of a blank row (the cell tier keeps the
row), suggestions sit on a panel painted just around their rows, and matches are one colour in the
file and the Search panel. Not done from the review: palettes' padding, the Review and Agents
panes, the light theme's mixed palette, and the table caption and minimap fixes.

A table's caption in a paper stays at the left with its table, where it was centred on the page.

Release 0.2.0.

Scrolling in plain cells, as under tmux over SSH. Each wheel step rewrote every row of the editor,
which shows as a scan over a slow link. The cell tier now has the terminal shift the band of rows
the editor spans with a whole-width scroll region, as vim and Claude Code do, and sends only the
lines coming into view and the columns that stay put, in the same synchronized frame. A test
measures it at under half the bytes of a redraw. `tmux_split_resize_and_pane_switch_keep_the_screen`
waited for a hint that only shows in an empty window, so it failed before this change; it now
waits for the status line's far end. Unverified: how it feels over a real SSH link.

Release 0.2.1, for the scroll region.

Round nine, from the second persona reviews: two ways edits landed in the wrong place. In Neovim
mode, `:e other` made Neovim's buffer another file while Cellix kept showing this one, so the
next key copied the other file's lines in and `:w` wrote them over it. After every key Cellix now
checks Neovim is still on the file in view in one window; a command that left it (`:e`, `:b`,
`:split`, `:tabnew`) is undone and the status says to open files with Ctrl+P and split with
Ctrl+\. In LaTeX, closing `\Cref{…}` at a line's end put the caret at the line's start, so the
next letters went into the line's first reference: snapping the caret now leaves a row's end and a
glyph's own place where they are.

Agent turns and plan compare. With Track Changes on, a second edit from the agent in the same turn
met "changed on disk" and a choice between losing it or the review. While the agent's suggestions
stand untouched, a later edit is now marked against the text from before its first one, so a turn
of edits piles up into one review. Once you've accepted or rejected some, the next edit still asks.
Compare Plan Versions diffs by line: lines that went are struck, lines that came are marked new,
and a line changed in place shows its words, so no mark spans a line and every one renders. Open:
a suggestion spanning paragraphs in a document still shows its markers.

One comment format. Every comment now carries what it's about: the selected words or the line,
for code, documents and plans alike. A comment on a line a change removed says so and counts in the
old version, where it used to point at whatever is on that line now. Plan comments name the plan,
not the private folder it's held in, and comments reach the agent in file and line order, each as
`file line N, on "quote": text`. `cellix comments` gives `quote` and `removed`, and the skill says
so. Open: plan comments still don't carry into the plan's next version as threads.

Rejections with a reason. Reject on a hunk in Review deleted the agent's code at one click and told
it nothing, and rejecting a proposed change only answered DIFF_REJECTED. Both now ask "Why reject
it?": Enter rejects, Esc keeps it, and a reason goes to the agent as a comment on the first line
that changed, with its quote, so it doesn't try the same again. Three tests that clicked Reject
now press Enter after it.

LaTeX across a project. A paper reads the `.bib` files it names with `\bibliography` or
`\addbibresource`, wherever they are, as well as any beside it, so a bibliography in `bib/` no
longer leaves every citation red and completion empty. TeX accents in a `.bib` read as letters
(`Cand\`es` is "Candès"). A paper an agent rewrites rebuilds its open PDF, as saving one does.
Open from the review: sections in `\input` files don't count toward the root's numbering or show
in the Outline, and there's no drawn references list.

A jump list. Go to Definition, Go to Line and opening a search result remember where they left
from; Go Back (Ctrl+-) returns there and Go Forward (Ctrl+Shift+-) repeats the jump, both also in
the palette. Without the kitty keyboard protocol a terminal sends Ctrl+- as Ctrl+/, which toggles
a comment, so there the palette is the way. A hundred places are kept.

Hover. Resting the pointer on code for 600 ms asks its language server about the symbol there,
and the answer (type, signature, docs, with Markdown's fences taken out) shows on a panel under
it until a key or a move away. Show Hover in the palette asks about the symbol at the caret.
Unverified: a real server's hover; only the parsing and the panel are tested.

Hover, checked against a real rust-analyzer: the panel showed but its text was hidden under the
panel's own paint (the test read cells, so it passed); the text is painted now, and Markdown's
rules and doubled blank lines are dropped. It shows the crate, the signature and the doc comment.

Shortcuts. Ctrl+A selected everything in every test, LaTeX source included; on a Mac in Ghostty,
Cmd+A is Ghostty's own (it selects the terminal's text), as Cmd+D was, so Settings' Ghostty card
now frees both. Added VS Code's everyday keys that were missing: Ctrl+Shift+K deletes the line,
Ctrl+Enter and Ctrl+Shift+Enter open a line below or above, Ctrl+Shift+L selects every occurrence,
Ctrl+Alt+Up and Down add a cursor, F8 and Shift+F8 walk a file's problems, Ctrl+N is a new
document, Ctrl+, is Settings, Ctrl+] and Ctrl+[ indent and outdent, and Ctrl+Tab switches tabs.
Several need the kitty keyboard protocol to be told apart; elsewhere they're in the palette.
Still different from VS Code: Ctrl+G opens Review (VS Code's Go to Line is Ctrl+L here) and
Ctrl+H stays Backspace.

The review's open items.
- A suggestion that runs across paragraphs shows as one: the page finds such marks over the whole
  document, tones what's inside them in every paragraph and hides their markers.
- A plan's comments carry into its next version: each is kept as a thread on the plan, marked as
  told since the agent heard it with the plan, and follows its quote into the new text.
- Sections in `\input` files count toward the paper: the page numbers on from where the file
  ends. The Outline lists an input file's headings in place (a click opens that file) and stops
  a title at its own closing brace, so `\label{…}` after it stays out.
- `\bibliography` shows "References" with every cited entry, numbered in citing order, as
  authors (year). title; a key the bibliography lacks says so in red. `\bibliographystyle` is
  hidden, where it ran the next line into raw source.
- Code's Outline lists its symbols from the language server (documentSymbol, nested ones under
  their parent), asked once per change, where it said "No headings here yet".
Unverified with a real server: document symbols.

Release 0.2.2.

Review with the agent as a second reader. Check my review, at the top of the Comments pane, asks the
agent to answer each of the reviewer's comments with `cellix reply <id> "Agree (<kind>): …"` or
`"Disagree (<kind>): …"`, and to add the comments a careful reviewer would, with the new
`cellix suggest <file> <line> <kind> "<words>" "<comment>"`. Verdicts read as "Claude agrees" or
"Claude pushes back" and set the comment's kind; suggestions show "Suggested by Claude · Evidence"
on an accent-edged card with Keep and Dismiss. A strip counts comments by kind (Clarity,
Correctness, Evidence, Structure, Reproducibility), a kind with none in the warning colour. Inside
Claude Code the mod submits the prompt as a turn; with an agent in the pane it's typed there; with
none, it waits for the one you start. The skill and README say how. Designed on the Claude Design
canvas first.

Round ten, from the third persona reviews.
- Plan comments reached the agent on Send back only for a plan inside the project: a plan held
  through an agent hook lives in a private folder, and its comments were named by file name but
  looked up by full path, so Send back went without them and the bar said "No comments". One
  helper names them now, and a test holds the plan outside the project.
- Approve carries the reviewer's comments, as a "Reviewer notes" section of the approved plan, so
  a caveat reaches the agent before the work rather than with the next prompt.
- Neovim mode: `:q!`, `:wq!`, `:x!`, `:qa!`, `ZZ` and `ZQ` ended the embedded Neovim and `:bd`
  emptied or `:bw` locked the buffer; `:w!`, `:up` and `:wa` didn't save. Each now goes through
  Cellix (save, close the tab, quit), and a buffer Neovim wiped is made again.
- Under tmux Cellix asks for modifyOtherKeys as well as kitty's flags, which tmux passes on with
  `extended-keys on` (the README's tmux setup now sets it), so Ctrl+Shift chords stop arriving
  unshifted. Unverified in a real tmux with the option set.
- Ctrl+R with Find open shows the replace row, where it was swallowed.
- Accept All and export leave inline code alone: marks inside a code span are code.
- Turning on Track Changes in a saved paper saves the `\usepackage{changes}` it adds, so the
  agent's next edit still arrives as suggestions.
- The rail is four columns, with icons 1.3 times larger, as the user asked.
Still open from the reviews: every agent edit as suggestions after a partial review, Replace All
under tracking, replies that carry rewrites, code actions, references in their own view, inverse
SyncTeX precision, warnings as Problems, `\input` inside a paragraph, natbib forms.

The writing loop, from the technical writer's round three.
- Every agent edit arrives as suggestions after a review has begun too: once some are accepted or
  rejected, the pending ones go back to what they replace and the agent's new text is marked
  against those decisions. Only typing of your own falls back to "changed on disk".
- `cellix reply <id> --suggest "<new words>"` answers a comment with a rewrite; the card shows
  "Claude suggests a rewrite" with Apply, which puts it in place of the quoted words as a
  suggestion to accept or reject, and Not this.
- Replace and Replace All under Track Changes write suggestions, not plain edits.
- Next Change and Previous Change (Alt+F5, Shift+Alt+F5, and the palette) walk the suggestions and
  say which of how many.
- A reviewer's `{>>note<<}` stays out of HTML and PDF exports.

Quick fixes. Quick Fix… (Ctrl+., the palette and the code's right-click menu) asks the language
server for code actions at the caret, with the problems on its line, and lists the ones that come
with their edits; picking one applies it across files as a rename does. Actions that only name a
server command are left out, since what they'd do can't be shown first. Unverified with a real
server.

LaTeX, from the paper author's round three.
- `\input`, `\include`, `\bibliography`, `\maketitle` and the like on the line after text start
  their own block, where they ran into the paragraph as raw source and broke numbering.
- A build's warnings are Problems too, as warnings: tectonic's `warning: file:line:` and LaTeX's
  undefined references and citations at their input line, so F8 walks them.
- The references list numbers as the PDF does: sorted by author for the plain styles, in citing
  order for unsrt, and a key with no entry is "[?]" rather than a number.
- Two authors cite as "Erdős and Rényi 1959", three or more as "et al."; two tests expected
  "et al." for two and now expect both names.
- Strike in a paper adds `\usepackage[normalem]{ulem}`, which `\sout` needs to build.
Still open: inverse SyncTeX lands a line high (a click resolves to its cell's top), natbib's
\citet and page notes, and brace-protected author names.

Nothing left behind. Closing the terminal or killing Cellix (SIGHUP, SIGTERM) ended the process
on the spot, so the workspace socket, the IDE lock and `--wait` comments were skipped; 315 stale
sockets had piled up in the temp folder, mostly from test runs, which end their child with a
hang-up. Those signals now end the loop the way Quit does. A closed Cellix also removes the plan
folders kept beside its socket, of which about 950 were left. The tour gains a shot of the
Comments pane with a verdict and a suggestion (50-review).

The Comments pane, against its design (ui-expert review of the 50-review shot).
- An agent's answer sits in an inset block under the comment, with its verdict on a chip
  ("Agrees" in green, "Pushes back" in the attention colour) beside who gave it, in smaller
  text, starting with a capital. The verdict test now looks for "Pushes back".
- Every card has a header: "You · Correctness" or "Suggested by Claude · Evidence".
- Check my review is a rounded button at the right of its row and the pane's only filled
  button; the note beside it reads "1 yours, 1 suggested". Ask agent is quiet, and Dismiss is
  a ghost until pointed at.
- The coverage strip is pills: covered kinds filled, missing ones outlined in the attention
  colour, wrapping onto a second row in a narrow pane.
- A suggestion's card has a softer accent edge and a faint accent fill, and no "Waiting"
  line, since the agent wrote it. Quotes drop math's dollar signs.
- Cards reach a third of a row past their text, with 12 px corners.
- The rail's tile is a square short of its row, and the changes badge is larger, ringed, and
  on the icon's corner rather than against the divider.
Not done: buttons still size in cells (so the cell tier can show their labels), so their
padding varies with the label; no robot icon on Check my review; no dashed edges, which the
renderer can't draw.

Unsent comments survive a restart. Code comments that no agent had taken yet lived only in
memory. On exit they're kept in the project's cache folder (out of the project), and the next
Cellix that serves the folder takes them back and says how many are waiting. A plan's answer
and `--wait` comments aren't kept, since they went where they belonged.

`TOUR_ONLY=50-review,02-code harness/x tour` retakes just those shots.

A stop that never finished. Catching SIGHUP and SIGTERM (above) let a stop land while
crossterm waited for input; crossterm retries the wait, and when the terminal had closed by
then it read the closed end forever at full CPU and never returned to the loop. A harness
script that stops Cellix and closes its terminal left 10 of 12 running; closing a terminal
window could do the same. A watch now ends Cellix two seconds after a stop if the loop hasn't,
clearing the socket, plan folders and IDE lock first and giving a live terminal back; the same
script leaves none. `a_stop_as_the_terminal_closes_still_ends_cellix` drives that through the
real binary. Found because about forty leaked tour processes, three spinning, starved the spell
tests; those processes are left for the user to end.

The 50-comments tour shot gives each step time to land: on a loaded machine its click and keys
arrived before the debug build had drawn the note.

From the user's Fedora machine (a Vitrine console, tmux inside, Ghostty outside).
- Ctrl keys were lost under tmux with `extended-keys on`, as the README sets it up. Cellix asked
  tmux for extended keys, and tmux 3.5+ sends them in the xterm form by default
  (`ESC[27;5;113~`), which crossterm can't read, so Ctrl+Q, Ctrl+S and the rest went nowhere.
  Cellix now asks only when tmux's `extended-keys-format` is `csi-u`, and the README sets it.
  `ctrl_keys_arrive_with_tmux_extended_keys_on` drives the real binary in a real tmux; by hand,
  Ctrl+S failed before and saves after.
- The graphics probe went to tmux too, which showed it as the window title (`Gi=31,s=1,...`
  in the status bar). Under tmux it's no longer sent.
- Over a console or SSH, tmux's TERM for the client can be a stand-in (`vt220` there) when the
  machine lacks Ghostty's terminfo. Cellix now also reads `client_termtype`, which tmux takes
  from the terminal itself (`ghostty 1.4.0`), before falling back to cells.
- The rail spaces its icons a row and a half apart (option A on the canvas, picked by the
  user), two rows in the cell tier, which can't place half rows. Each icon owns the rows nearest
  its middle for clicks and hover. The tour's rail rows moved with it.
Unverified on the Fedora machine itself: it has no SSH server and its console was in use, so
0.3.0 waits for the user to install there.

Release 0.3.0.

The open items, closed for 0.3.1.
- Under tmux over a console, a line showed under every row of the editor. The console passes
  only rows and columns, so tmux made up a 16 by 32 cell, and images sized for it were
  letterboxed into Ghostty's real cells. With passthrough on, Cellix now asks the terminal
  outside tmux for its cell size (checked by hand: the answer comes back through tmux), and a
  size the terminal gave isn't overruled by the window's on a resize.
- Inverse SyncTeX lands on the line clicked: a click counted from its cell's top, most of a
  line above the pointer, and now counts from its middle.
- natbib: `\citet` reads `Knuth (1984)`, notes print as `[see Knuth 1984, p. 3]`,
  `\citeauthor` and `\citeyear` give their part. Up to two options are read for any command.
- Names cite whole: `{World Health Organization}`, `van der Berg, Jan`, `Jan {van der Berg}`.
- Callouts with a title of their own (`> [!tip]- Keep it short`) and Obsidian's kinds show
  their title on the page and keep it in HTML and PDF exports.
- Saving a document with suggestions still open says they stay in the file as marks.
- A comment on a sentence written twice stays on its copy while the copies stay as many, and a
  reworded quote takes its new words, so the card shows what the text says now.
- Track Changes is remembered per document, in the project's cache folder.
- One line of a change can be undone: right-click a changed line in Review for Remove This Added
  Line or Put This Line Back, beside Comment.
- Approving a Codex plan with changes or notes starts one more turn that hands them over.
- Quick fixes and the Outline were driven against a real rust-analyzer: Insert explicit type
  applied, and the Outline listed the file's symbols. Quick Fix's first item is now ready for
  Enter, as in VS Code.
- The Comments pane: suggestions have dashed edges (panels can be dashed now), buttons are as
  wide as their words plus the same room either side in pixels, and Check my review carries the
  agent's icon.
- Ctrl+Shift chords under tmux with `extended-keys-format csi-u` are driven in real tmux.
- Harper builds optimised in debug: unoptimised, the spelling tests took about a minute and
  failed their deadline on a loaded machine.

Release 0.3.1.

Plan comments carry into the plan's next version as threads. When an agent sends a plan again,
each thread on it moves onto the new text, with its replies and whether it was resolved. One whose
words the new version dropped stays open with its quote, struck through on a quiet bar, and a
"Removed in version N" pill; clicking it opens what changed since the version it was made on, and
an agent reading it hears which version it was about. The Comments pane opens when a plan arrives
with comments on it, and a plan still open from before reloads. The tour has a `50-plan-versions`
shot. Unverified: a real agent sending a third version, where a thread made on version 2 compares
against `Plan v2.md`.

A paper split over `\input` and `\include` counts as one. The word count reads the paper from its
root with each file it takes in read in place, the file being edited as it stands unsaved, and the
status says "words in the paper" when it took in more than one file; saving or an agent's edit
counts every open file again. Looking for what else only saw the file in front found two more,
both fixed: Track Changes in a file the paper takes in added `\usepackage{changes}` nowhere (that
file has no preamble), so the build failed on the marks; the package now goes in the root's. And
an agent rewriting such a file rebuilt the open PDF only when that file was in front; any open
file of the same paper does now. Left as they are: spelling checks the file in front, which is
what it shows, and files the paper takes in that aren't open aren't watched for an agent's edits.

pdflatex without tectonic. `harness/x tex` runs four ignored tests of the real binary in a PTY,
with tectonic hidden from its PATH (and latexmk too in the plain case): Build PDF on a paper over
two files that cites a `.bib`, and Export to PDF on a Markdown note. They check the PDF, that
BibTeX ran, that the log has no undefined citation or reference, no "Rerun" and no `??`, that
SyncTeX was written, which engine ran, and that an export leaves nothing behind. CI runs them on
Ubuntu with only `texlive-latex-recommended texlive-bibtex-extra latexmk`. Reading the code
against what TeX Live does found two bugs, fixed:
- Exports built from a hidden `.name.export.tex`. TeX Live won't write a file whose name starts
  with a dot (kpathsea's `openout_any = p`, as its source reads; not seen here), so pdflatex
  couldn't open its log and an export without tectonic would fail. The source is now `name.cellix-export.tex`, briefly visible while it builds;
  everything it makes still goes afterwards, `.bbl`, `.blg` and `.toc` now included. The test
  that held the hidden name now holds that the name doesn't start with a dot.
- The export loaded TikZ, tikz-cd and ulem into every document, which a small TeX doesn't have.
  Packages past amsmath, graphicx, hyperref and geometry now load only when the body uses them.
Unverified: the tests have not run anywhere with pdflatex. This Mac has only tectonic, and the
same tests passed against tectonic here as a probe of the driving, the waiting and the cleanup.
Their features stay unmarked until the CI job passes.

The tex job ran the four latexmk and pdflatex tests against TeX Live on CI and all passed, so
those features are marked passing.

## 2026-11-04: Windows

Cellix builds for x86_64-pc-windows-msvc, with a CI job on
windows-latest that runs clippy and every test, and a Windows zip in the release.
- Agents reach it through `cellix` as before. Where there are no Unix sockets the workspace
  socket is a port on 127.0.0.1, named with a 128-bit token in a file where the socket would be;
  a connection without the token is dropped. Unix keeps its socket unchanged. The transport is
  plain std, so `agents_reach_cellix_over_loopback_with_its_token` proves it here and on
  Windows, with `a_loopback_note_nothing_answers_is_replaced` for a note left behind.
- Gated with a fallback, never a panic: the session host (programs in the side pane run on
  ConPTY under Cellix and end with it), hang-up and stop signals, interrupting a running
  notebook cell, the pane's check for an orphaned parent, terminal queries (so no Kitty
  graphics, cell size or background colour; Windows starts on the cell tier), and pictures from
  programs in the side pane, which ConPTY doesn't pass on. A shell tab closes without asking,
  since a console can't say what's in front of it.
- Fixed for Windows: `\\?\` canonical paths (dunce), file URLs (`file:///C:/x`), the PATH and
  `CLAUDE_CODE_PLUGIN_DIRS` separator, `.exe` and `.cmd` on the path, picture links written
  with `/`, a venv's `Scripts/python.exe`, PowerShell for the Terminal tab, `rundll32` to open
  pages, and HOME taken from USERPROFILE when unset.
- crossterm refuses the Kitty keyboard flags on Windows, and that error ended Cellix before its
  first frame. They're asked for only elsewhere now; Windows keys come as console events.
- The PTY tests run the real binary through ConPTY on the runner: it starts, draws, follows a
  resize, takes a bracketed paste, restores the terminal, holds `--wait`, and `cellix comments`
  and `open` reach it over the loopback socket. They keep the PTY's master (dropping it closes
  a pseudoconsole), answer ConPTY's cursor query until Cellix draws (a first answer can be
  lost), and wait 40 seconds on Windows, not 10.
- Unix only among the PTY tests: hang-up and stop signals, the pixel tier (Windows asks the
  terminal nothing), Cmd and Ctrl+Shift keys sent in the Kitty keyboard protocol, and the mouse
  click test. That click, an SGR report written to ConPTY, never reached Cellix on the runner
  though keys did. Next: find out whether clicks arrive in a real Windows Terminal, and if not,
  why crossterm doesn't see ConPTY's mouse events.

Unverified: never run in a real Windows Terminal or WezTerm, only on the CI runner through
ConPTY, so mouse input on Windows is the biggest unknown. The Claude Code mod, Codex and Gemini
hooks, and `CLAUDE_CODE_PLUGIN_DIRS` taking `;` on Windows are untried there.

## 2026-11-04: Review round after the Windows merge

Four reviewers read everything since 0.3.1: a Windows systems developer, a Rust developer on the
plan, LaTeX and release work, a paper author on pdflatex, and a technical writer. Fixed here:
- The README's Linux install line failed with GNU tar ("cellix: Not found in archive"), proven in
  a Debian container: the archives hold `./cellix`. It now extracts `./cellix`, makes
  `~/.local/bin` and fails on a 404, in the README, the release notes and the six published
  releases' notes.
- The release workflow: a failed tap or crates.io step could never be redone, since a rerun saw
  the release and skipped both. Each now does only what's missing, and a run by hand on main with
  `finish` redoes them; automatic runs don't, so nothing reaches crates.io unasked. A run by hand
  on a branch, or a tag on a commit not on main, only builds. A tag left on another commit stops
  the release instead of publishing one commit's archives under another's tag.
- `harness/x bump` works out every edit before writing, so it can't leave a half-bumped tree.
Unverified: the workflow changes have not run yet; the next release, or a run with `finish`, is
the test. Next: the LaTeX, plan-thread and Windows findings, each being fixed on its own branch.

## 2026-11-04: The prompt hook timed out, and the Comments pane didn't scroll inside Claude Code

An early user on macOS saw "UserPromptSubmit hook [cellix hook prompt] timed out after 30s" on
every prompt, in a project with data files. Claude Code drops what a timed-out hook prints, so the
comments it takes never reached the agent, which is why they kept running /cellix review instead.
The hook checkpoints the tree first, and the checkpoint deleted its private index and ran
`read-tree HEAD` and `add -A` each time: every file was hashed again, and every untracked file
written into `.git`. Timed here, 200 MB of untracked data took 10.5 s the first time and 1.2 s
each time after; a few GB is past 30 s on every prompt. Now:
- The checkpoint index is kept between checkpoints, first seeded from the repository's own index,
  so git hashes only files whose size or time changed. One it can't read is rebuilt, and a lock on
  it older than ten minutes is cleared.
- Untracked files over 10 MB stay out of checkpoints, so they're neither hashed each prompt nor
  copied into `.git`. Review won't show a change to such a file, and Undo Turn won't touch one.
- The hook runs `cellix checkpoint` in a process of its own and waits for it at most 3 s.
Their other report: the wheel didn't scroll the Comments pane inside Claude Code. Claude Code's
scroll event has no position, and a wheel without one always went to the file. It now goes to the
side pane when the pointer last moved over it.
Unverified: not yet tried on their project; the hook test proves the checkpoint still lands from
the process the hook leaves it to.

## 2026-11-04: plan threads across versions, reviewed

Fixes to carrying a plan's comments into its next version (458a767), after review:
- A reworded quote no longer counts as the same words when a plan is sent again. Only exact or
  rewrapped words carry a thread along. Reworded words keep the original quote and show a quiet
  "Changed in version N" pill, which opens the comparison. A negation or a number in or out is
  never a reword, anywhere a comment follows its text.
- Approving a plan resolves its threads and keeps them with the approved copy in `.cellix/plans`.
  The hook then puts that plan's versions away in `Approved N`, so the next plan starts at
  version 1 with none of them.
- Carrying saves the moves of resolved threads too. A dropped comment tells the agent its
  version, not a line it isn't on ("made on version 1; version 2 dropped these words"). Its card
  opens the comparison even when no plan is waiting.
- Per the ui-expert: quotes on changed and removed cards stay readable, the strike sits
  mid-letter, Changed has a neutral pill and Removed the warning one, and both end in "›". A
  pill too wide for its header's row goes on a row of its own.

Unverified: the pill on a row of its own only shows in the cell tier. In pixels the pane is
never narrow enough beside the editor, and below about 90 columns it stacks under the editor
at full width. The hook's `Approved N` move is tested through the real hook, but not with a
live Claude Code session.

## 2026-11-04: LaTeX build output, reviewed

Fixes from a review of what Cellix does with a build's output.
- A failed Markdown export no longer replaces the last good PDF: pdflatex and latexmk write
  one despite errors, so it's renamed only when the build worked, and deleted otherwise. Its
  errors point at the hidden `.cellix-export.tex`, already gone, so they're said ("The PDF
  wasn't made: …") with nothing to jump to, and a jump to any missing file only says where.
  They aren't mapped back to Markdown lines; there is no line map yet.
- An export loads tikz-cd, ulem, booktabs or xcolor for what it emits, not for code shown in a
  code block.
- Undefined-reference warnings go to the file TeX was reading, from the log's `(./file` … `)`
  stack, with lines TeX broke at 79 characters joined back first.
- Warnings count once, from the last pass: the .log when there is one, plus tectonic's own
  `warning:` lines; summaries ("There were undefined references") don't count again.
- `\includegraphics` on the page of a section file looks in the root's folder, then each
  `\graphicspath` folder, as LaTeX does.
- Forward search matches the source by its whole canonical path, so `ch1/intro.tex` and
  `ch2/intro.tex` no longer collide.
- The pdflatex fallback runs Biber for biblatex (`\abx@aux` or a `.bcf`), runs again while the
  log asks ("Rerun to get…", "Label(s) may have changed"), up to three more times, and says
  when BibTeX or Biber fails, with its output in the Log tab. Both tools are looked for beside
  a pdflatex named by path first.

Unverified here: the new TeX Live tests in `tests/tex.rs` (Biber, `\include` reruns, a missing
.bib, a latexmk warning count, a failed real export) run only in CI's `tex` job, so their
features stay `"passes": false` until it passes. Next: map an export's errors back to Markdown
lines.

## 2026-11-04: Verify in parallel worktrees

Every worktree wrote one shared `verify.log` and one `verified` stamp, so agents verifying at once
overwrote each other's results ("did not pass" on tests that passed) and removed each other's
stamp, and the commit hook refused verified work. Each worktree now keeps its own log in its git
folder, and every tree that passes gets a stamp of its own under `cellix-guard/verified.d`, kept
a week. The hook still accepts the old single stamp, which worktrees on an older harness write.
Unverified beyond this commit's own gate: two worktrees verifying at the same moment.

## 2026-11-04: papers split over files, read as LaTeX reads them

Fixes from a review of how a paper over several files is handled.

- A section's root is a whole document that really `\input`s or `\include`s it, looked for in its
  folder and up to three above, no higher than a repository's top. It was the first
  `\documentclass` file one folder up whose text mentioned the section's name, and since tracking
  changes writes `\usepackage{changes}` into the root, that could edit an unrelated file.
  `% !TeX root` is read in any case.
- One paper walk, `cellix_doc::paper`, serves the word count, outline and numbering: `\input name`
  without braces, `name.tex` tried before `name`, inputs anywhere on a line, a file that takes
  itself in read once. The word count reads other open files of the paper unsaved, and keeps the
  rest by size and time of change instead of reading them on every keystroke.
- The changes package goes in unless a real, uncommented `\usepackage` loads it, before the real
  `\begin{document}`.
- Build PDF first saves the paper's other open files with unsaved edits.
- Cite reads the `.bib` files the paper names, `\addbibresource[options]` included, from its root.

Unverified: the outline still reads `\input` files from the open file's folder, not the root's,
and the numbering and outline read other files from disk, not their unsaved tabs. The root search
stops at `.git`, not at the workspace, since `root_of` doesn't know the workspace. No UI changed,
so no tour was run.

Harness: `harness/x verify` keeps its stamp and log in the shared git dir, so a verify started in
another worktree clears the stamp and overwrites the log mid-run, failing verifies that passed.
Next: give each worktree its own stamp and log.

## 2026-11-04: Windows review fixes

A review of the Windows port found these, all real, now fixed with tests that run on every
platform unless said:
- The loopback socket proves both ends. The note holds a client token, a server token and the
  pid; a client gives its token and asks nothing until Cellix answers with its own, so whatever
  takes the port after Cellix has gone can't feed an agent comments or approve a plan. A note
  whose process ended, whose port refuses or times out, or that answers wrongly is removed.
  Connecting and proving take 2 s at most and a reply 5 s, on Unix too; only `plan` waits.
  A Cellix removes only its own socket file, and a failure to start the server is said.
- The side pane's PATH is joined with `join_paths`, not `:`.
- Closing the console runs a `SetConsoleCtrlHandler` that stops the loop and holds Windows off
  for up to 4.5 s while unsent comments are kept and the socket and IDE lock go.
  `closing_the_terminal_keeps_unsent_comments` passes on the Windows runner and here.
- Paste: ConPTY strips the bracketed paste markers and hands Cellix keys, so a paste's Enter
  took a word completion. `pty_bracketed_paste_inserts_verbatim` now compares the saved file
  (it only waited for a word, so it passed anyway). Keys that type, read whole in one batch,
  become a paste when they carry on after Enter or Tab, or sit between markers sent as keys.
- Socket notes on Windows live in `%LOCALAPPDATA%\cellix\sockets`.
- Codex and Gemini run hooks in PowerShell on Windows (read in their source), so `cellix setup`
  writes the bare `cellix hook ...` there. Claude Code's hooks use Git Bash and are unchanged.
- Notebooks prefer the `py` launcher on Windows, and shares get `file://server/share` URLs.

The mouse: through Microsoft's own ConPTY (NuGet `Microsoft.Windows.Console.ConPTY`, the
conpty.dll and OpenConsole.exe Windows Terminal ships) all the PTY tests pass on the runner, the
click test included, where through the built-in console the click never arrives. So clicks
should work in Windows Terminal, and not in the old console host. The windows job keeps that
as a step, "PTY tests through Windows Terminal's ConPTY", pinned to 1.25.260930003 (a NuGet
version can't change once published), and it fails the job like any other. By default the
mouse test stays ignored on Windows.

Unverified: nothing has run in a real Windows Terminal; the Codex and Gemini hooks and the
Claude Code mod are untried on Windows. Next: try it in Windows Terminal itself.

## 2026-11-04: Paths in comments use / on Windows too

After the merges, CI on Windows failed `an_approved_plans_threads_stay_with_it_and_the_next_plan_starts_clean`:
an approved plan's thread was filed under `.cellix/plans\001.md`. Cellix named workspace files
with the platform's separator, so on Windows a thread or comment said `docs\x.md` while git, the
agents and saved threads say `docs/x.md`. Paths in comments and threads now use `/` everywhere,
which Windows reads as well.

## 2026-11-04: The Preview

The side pane's tab for a PDF or an image was called "PDF" even for an image. It's Preview now,
twelve columns like the other views' name, padding and ×. With an image, its texts say "image"
("Reading the image…", "This image can't be shown.", "Image ready:" on the cell tier), its bar
names the file instead of counting pages, and a click on it no longer says to build with
SyncTeX. Build PDF, Export to PDF and Show in PDF keep their names: they are about PDFs. The
tour has a new shot, `90-preview-image`. Next: panning a zoomed page.

## 2026-11-04: Panning the Preview

Zoomed past the pane, a page was cut off at the right with no way across. The Preview now keeps
a sideways offset. A left press on the pages pans once it moves three quarters of a column or a
row, so a click without a drag still goes to the source through SyncTeX, now on release
(`clicking_a_pdf_goes_to_its_source` sends the release it always should have). Shift+wheel and
the new `ScrollLeft`/`ScrollRight` kinds go across; elsewhere those kinds are ignored and don't
clear messages or spoil a scroll on its way, so a trackpad's drift can't jitter the editor. Thin
thumbs like the editor's sit over the bottom and right edges and can be dragged. "Drag to move
around · Shift+scroll goes sideways" floats at the bottom the first time a page is too wide,
once a run, for five seconds or until the first pan. A press in the Preview gives it the keyboard,
where Space and other plain keys do nothing and the arrows move the page; Esc gives it back, and
Esc during a drag puts the page back. Inside Claude Code the mod's pointer down, move and up
carry positions, so a drag pans there too (tested through `pane::translate`).

The tour's new shots (`91-preview-zoomed`, `92-preview-panned`) and a design review caught a
moved page drawing its outline over the file beside the pane, a page that fits still moving up
and down (scrolling now stops when the last page's bottom is in view), pages sliding up against
the bar's text, and thumbs touching the corners; all fixed. The thumbs keep the editor's colour.

Unverified: a real terminal's sideways wheel and Shift+wheel (crossterm's kinds are tested, a
live trackpad isn't), and the pan inside a live Claude Code session. Pages don't show on the
cell tier, so neither does panning. Next: maximizing the pane and an editor.

## 2026-11-04: Maximize

The side pane's tab row ends with a maximize-2 button after its +, which makes the pane fill
everything right of the activity rail. Maximized, it shows minimize-2 on the selected chip, with
the tooltip "Restore or double-click the tab", and the status bar says which file is hidden. A
double-click on one of the pane's tabs does the same. A double-click on an editor's tab maximizes
that editor, hiding the sidebar, the other split panes and the side pane, and again restores.
The palette has Toggle Maximized Side Pane and Toggle Maximized Editor; no keys. Double-clicks
use the existing time and place window. The icons at the top right stay put. What's shown is
compared before and after every event and every agent call: a hidden part asked for (the pane
behind an editor, the sidebar, a file behind the pane, Review, Settings, a split) restores first.
Kept while running only. The tour has `93-pane-maximized`, `94-pane-restore-hover` and
`95-editor-maximized`; a design review made the button's chip square and its icon the strip's
size, gave it one step from the +, moved the tooltip clear of the bar's text, and named the
hidden parts in the status bar for an editor too.

Unverified: in the tour's debug build, a double-click on the tab of a Markdown page with a
picture didn't register, though it does in a test and on a code file. Multi-clicks are timed when
each press is handled, so a slow frame between the two presses can split them; a release build
wasn't tried. Next: dragging tabs.

## 2026-11-04: Dragging tabs and the side pane

A press on a tab becomes a drag after two columns or a row, so clicks and double-clicks are
unchanged. A file's tab dropped on the tab bar moves there, with a 2 px accent bar where it would
go; on the left or right half of a pane it opens a new pane on that side and moves there, the old
pane showing its neighbour; in the middle third of another pane it shows there. Cellix has one tab
bar for all panes, so "another pane's tab bar" became that middle drop. Panes only split side by
side (`Command::SplitPane` appends one, and `pane_area` divides the width), so there's no bottom
split; a split needs 20 columns a pane. The side pane's tabs move along their own row, views among
views and sessions among sessions. Dragged over the bottom half of the file, a side pane tab docks
the whole pane under it, reusing the narrow layout (`dock.under`), and on the right edge puts it
back; kept while running. While dragging: a ghost chip at the pointer, an accent wash at about 14%
with a 2 px border where it would land (a new `Paint::Wash`, since panels are opaque), and the
status bar saying what dropping does. Esc cancels. On the cell tier the bar is `▏`, the wash tints
cell backgrounds and the ghost is a label.

The tour has `96-tab-drag-split`, `97-tab-drag-reorder`, `98-pane-drag-under` and
`99-pane-under` (a new `press`/`hold` step in `term.py` holds a drag mid-way). A design review
moved the insertion bar off the sidebar's edge, put the ghost below and right of the pointer so
it hides nothing aimed at, and made the docking wash cover where the pane will really stand.
Left as they are: the dragged tab isn't dimmed, the minimap shows through a split wash, and the
Preview fits its page to the width when docked under, so a tall page scrolls there.

Unverified: drags in a real terminal and inside Claude Code beyond the tour's headless terminal.
In the tour's debug build the frame after a whole-layout change can take over a second.

The pane's maximize button looked missing in `91-preview-zoomed` and `92-preview-panned`. Those
two shots were taken before Maximize was built and never retaken; retaken, the button is there.
A test now holds it drawn while the Preview is zoomed, panned and hovered.

## 2026-11-04: Installs no longer hang on a question

Reported: after installing for Claude Code and then terminal-browser from Integrations, Settings
said another install was still running long after it had finished, until Cellix was reopened.
terminal-browser's installer ends with `terminal-browser setup`, which asks on `/dev/tty` whether
to install its agent skill. Cellix runs installs from inside its own terminal, so the question
went to Cellix's screen and waited for a `\n` that raw mode never sends (Enter arrives as `\r`).
The tool was installed, but the shell never exited. Installs now run in a session of their own
with no terminal (`setsid`), so a question like that takes its default answer and `sudo` fails
rather than waiting for a password. This covers every install button: Integrations, the
terminal-browser card and the language server card. The test runs an install from inside a
real pseudo-terminal, re-running the test binary there; without the fix it waits out its 30 s.

Unverified: a real terminal-browser install from Settings after the fix. Windows is unchanged.

## 2026-11-04: The pane inside Claude Code over SSH

Reported: with Claude Code over SSH, `/cellix` blinked on every mouse move, and clicks were slow
or lost. Over SSH the pane sent a PNG of the whole pane on every change, and a pointer move
changes what's hovered, so each move sent a picture across the connection and the swap showed
as a blink. Input also queued: the mod ran one `cellix pane-send` per event, one after another,
so a click waited behind every move before it, and Cellix drew a frame for each.

Now, over SSH (or with `CELLIX_TIER=cells`), the pane is the cell tier: `cells` lines that the
mod draws as Claude Code's `Raster`, so Claude Code writes only the changed cells to the
terminal. Each cell is a width-1 BMP character: icons go as their cell-tier drawing, `☰` as
`≡`, and other wide characters as `?`. A Raster has no cursor, so the cursor's cell is drawn
inverted. The mod gathers input and sends one batch per `pane-send`, with what piled up during a
send going in the next; the input module and Cellix both keep only the last of a run of moves,
and Cellix takes everything waiting before it draws. The PNG path is gone.

Tests: a CLI test runs the real `cellix pane` with `SSH_CONNECTION` set and drives it with a
batch through `pane-send`; unit tests cover the cell encoding, icons and the cursor, batches and
moves; the mod's tests cover drawing cells, repainting them in place and batching. `harness/x
snapshot` takes `CELLIX_TIER=cells` and `CELLIX_PANE=1`, and `select:`/`ask:` views. A ui-expert
review of the cell frames raised things the cell tier already had, left for a design pass: the
top bar's glyphs (`◆ ◍ ◎` read badly, and `≡ ◆ ± ◎ ×` are ambiguous width, so CJK-wide terminals
draw them two cells wide), bold that a Raster can't show (headings look like body text), a
selection bar that reads like a line of the file, and a code block one cell short on the right.

The ledger's `pane-remote` entry, which named `a_remote_frame_travels_as_a_png`, now
names `a_remote_frame_travels_as_cells`: frames still travel by value over SSH, as cells now.

Unverified: a real Claude Code session over SSH. Pictures and PDFs show as on the cell tier.

## 2026-11-04: Ask Agent inside Claude Code

Reported: in the pane inside Claude Code, a selection's Ask Agent opened Cellix's comment bar,
which is for an agent running in Cellix's own side pane; inside Claude Code the agent is the
conversation beside it. Now Ask Agent there sends a `fill` line, and the mod puts
`@<file> line <n>, "<selection>": ` into Claude Code's prompt box at the cursor, unsent, through
`$.prompt.fill`. The `@` mention makes Claude read the file when the person sends it. The
selection collapses so the bar goes, as a ui-expert review asked, and the status bar says
`"cache" is in Claude Code's prompt. Press Esc to ask there.` A mod can't hand the keyboard to
the prompt, so Esc, which already returns it, is the step named. Outside Claude Code, Ask Agent
is unchanged.

Unverified: in a real Claude Code session, that the fill lands while the pane holds the keys.

## The install test on Linux (2026-11-05)

CI failed 0.4.1 on Linux in `an_installer_that_asks_on_the_terminal_still_finishes`. The test's fake installer opened the terminal with `exec 3<>/dev/tty`, and dash, Linux's `sh`, ends the whole script when `exec` can't open a file, so the install failed instead of going on. macOS's `sh` is bash, which carries on, so it passed here. The fix to installs was right; the test was wrong. It now tries the terminal on `true`, whose failed redirection is only false in both shells. Checked by hand under bash and dash: with a terminal it asks and waits, without one it finishes. Next: CI on Linux, then the 0.4.1 release.

## The Settings tab closes like any other (2026-11-05)

From the user: Ctrl+W and a middle click closed every tab but Settings. Settings is a page drawn over the editors with a tab of its own, not a tab in the list, so Ctrl+W fell through to the file hidden behind it and closed that, and the page ignored every button but the left. Ctrl+W now closes Settings while it shows, and a middle click on its tab closes it. Test `the_settings_tab_closes_like_any_other`. Unverified in a real terminal. Not released; it goes out with the next version.

## Quick Open in big trees, and the pane that stayed on "Starting Cellix…" (2026-11-05)

From the user: Quick Open from the "Open a File" button took a long time in a big tree on a Mac, and on Amazon Linux `/cellix` stayed on "Starting Cellix…".

Quick Open walked the whole folder on the main thread before it showed anything. It now opens at once and a walk on another thread hands it files every 50 ms, labelled there so the main thread only appends; a small folder still opens whole within 100 ms. Typing rescored every file with a lowercased copy of each name; scoring no longer allocates, and a query that only grew rescans just what matched. In a 300,000-file folder on this Mac the palette shows in 0.1 s and each key takes about 10 ms. The walk stops at 500,000 files and says so.

Reproduced on Amazon Linux 2023 in Docker with Claude Code 2.1.318 and tmux, there were four ways to "Starting Cellix…" or a blank pane:
- An upgrade left the plugin the old Cellix wrote. With a 0.4.0 plugin and the 0.4.1 binary over SSH, the mod ignored the new `cells` lines for good. This is likely what the user saw after upgrading. The session-start hook now rewrites changed plugin files, which Claude Code picked up in the same session, and the mod sends `CELLIX_MOD` so a pane can tell an older mod to restart Claude Code.
- Claude Code started without `cellix` on its PATH: the spawn rejected, the rejection skipped the code that says why, and Claude Code still said "Cellix is open". Setup now writes `CELLIX_BIN`, the mod and hooks fall back to it, and a spawn that fails says how to fix it.
- `git status --untracked-files=all` ran on the main thread before the first frame: 1.1 s for 300,000 untracked files, longer in bigger repositories. It now runs with the rest of the disk look in the background; the branch stays synchronous.
- Outside Ghostty and kitty, as in tmux, the pane sent pictures Claude Code can't show and stayed blank. It now picks cells from the environment it inherits.
Also, the pane took a parent of process 1 to mean Claude Code was gone, which is wrong in a container where Claude Code is process 1.

Checked in the container with the new build: the upgrade case draws in the first session; off the PATH it starts from `CELLIX_BIN`; with neither it explains; tmux without SSH draws as cells. Unverified: the user's own Mac tree and Amazon Linux machine, and whether Claude Code reloads a rewritten mod everywhere as it did here. Next: release 0.4.2 with these and the Settings tab fix.

## Pictures for the pane over SSH and in tmux (2026-11-05)

The user wanted the pane inside Claude Code over SSH to look as it does locally, not fall back to text, and for Cellix to set up what it needs, as Claude Code does.

Measured first with a throwaway probe plugin in real Claude Code 2.1.318 over real SSH into the Amazon Linux container, with a pty "terminal" on the Mac answering like Ghostty and recording every graphics command:
- Pictures by value (`png`) cross SSH; a source equal to the last drawn sends nothing; Images drawn under keys that move keep their uploads (a shift by one row uploaded one picture and deleted one).
- Inside tmux Claude Code needs tmux's global `allow-passthrough` and `focus-events` on when it starts. A pane option, a SessionStart hook or setting them while it runs all came too late; set before it starts, with no reattach, works. tmux also needs a UTF-8 client, or it mangles the placeholder characters.

Built: `rows` lines, a PNG per row named by a hash of the row, sent only when the mod lacks the name; the mod draws Images keyed by name and checks once with a self-blit whether pictures show, falling back to cells on "draws its alt". Palette PNGs (rows were 18 KB as fast RGB, 5 KB as palettes), half resolution by nearest pixel for bursts of whole new pages, nothing until the size is known, and the row key now cuts tall shapes to the row's reach, so rows under a page's sheet keep their names when scrolled (8 of 43 kept before, 33 after). `cellix setup claude` writes the tmux lines; the hook and the pane set them on a running server; a session started before says to restart. `CELLIX_PANE_LOG` logs what crossed.

Measured end to end over SSH and tmux: open 537 KB, click 21 KB, a letter 28 KB, hover nothing, five scroll notches 280 KB plus tmux's placeholder text, first picture 65-75 ms after a click or wheel on a local link. tmux's own grid drawn from the recorded pictures shows the pane exactly, rows in order. Unverified: a real Ghostty window, a slow real link, and the user's own server; the row pictures assume the cell size the pane's tty reports.

The design review found the rows soft: they were drawn for 16x32 cells and stretched to the terminal's 17x37. That size came from the test path, `docker exec`, which carries no pixel size, so tmux fell back to 16x32. Over real SSH from the Mac (OrbStack reaches the container by address) with a real TERM, tmux knows the client's 17x37 cell and the pane's tty reports it; rows then arrive at exactly 89 x 17 by 36 and nothing is scaled. The pane now reads the cell size again on each resize, since a font size change in the terminal resizes the pane, and the cell size is part of a row's name, so pictures drawn at the old size aren't kept.

## Workspaces without tmux, and dark mode that follows the terminal (2026-11-05)

From the user: can Cellix do what tmux does over SSH, keep work in the background, with a bar of tabs along the bottom to move between Cellix workspaces, and resume, even after a reboot? Decided with them: both ways stay possible (tmux still works), a Resume button after a reboot rather than starting agents on its own, a prompt where the server ends programs at logout, design A for the bar (tiles that say what each agent is doing), and the bar only with two or more workspaces. Designed first on the Claude Design canvas (WsBarA/B, Resume, Linger, Running).

Built:
- Session hosts keep their output in a log beside the socket, serve every Cellix that connects (whoever typed last sets the size), and tell the program its socket (`CELLIX_HELD`); agent hooks leave the agent's own session id there. A host that dies without finishing is put aside, not deleted.
- A stopped tab comes back with its last output dimmed and a card: Resume (`claude --resume`, `codex resume`), Start Fresh or Start Again, Close. Gemini CLI isn't resumed: its resume flag wasn't checked.
- `state.json` per folder keeps tabs, cursors and unsaved edits (hot exit), written when they change and on exit; Don't Save on Quit keeps tabs but not edits.
- `Workspaces` holds an app per folder, each with its own socket and IDE port, and draws the bar on the bottom row; it opens every folder with something running or stopped that no other Cellix has open. Quit asks each workspace in turn; closing a tile asks as Quit does. Cellix started at home offers what's running first. `cellix ls`. Ctrl+Alt+PageUp/PageDown.
- On Linux over SSH, a card offers `loginctl enable-linger` when logind kills user processes at logout (`CELLIX_LOGOUT_STOPS` forces it, for the tour).

Also from the user, mid-way: after the Mac turned dark, Cellix over SSH stayed white. Cellix asked the terminal's colour only at start, and inside tmux tmux answered from what it learned at attach. Probed in real tmux 3.6a over SSH: tmux keeps the terminal's answer to a passthrough OSC 11 to itself, but that answer updates tmux, and tmux's own answer afterwards is current. Cellix now asks again on focus-in, and inside tmux asks past tmux first, waits for the passthrough cell size answered after it, then asks tmux. Checked in the container through real tmux: white at start, dark after the terminal turned dark and focus came back.

Verified on Amazon Linux over real SSH: Claude Code in the side pane kept running after the connection dropped (`cellix ls` listed it), a container restart left it stopped, the next `cellix` showed the card, and Resume started `claude --resume <its id>` with the conversation back. Two ui-expert rounds on the tour pictures (100-104); fixed chip size and spacing, one card rule, the command well, the first screen's frame, rows and path column, dimmed stopped output.

Unverified: a real Ghostty window for the bar and cards; "Stopped when <machine> restarted" (a container restart keeps the host's boot time, so it said "Stopped without finishing"); the logout card on a real systemd server; Codex resume end to end. Not done: "Open a Folder…" on the first screen; mode 2031 colour reports (crossterm drops them). Next: the user tries it on their server, then release.

## Bar clicks in Ghostty, folder double clicks, dimmed ignored files, a quit button (2026-11-06)

From the user, on a Mac in Ghostty: only the top of a workspace tile took a click; a folder in the tree sometimes opened and closed at once; and two requests, gitignored files half-hidden as in VS Code and out of search, and a close button at the top right that quits Cellix. Then release without asking.

- Bar clicks: Ghostty draws the last row down into its bottom padding (`Bleed.drop`, half a row with the user's `window-padding-y = 8`), so the lower half of a tile lies past the grid's last row and its clicks counted as below the screen. `Workspaces` now takes any row past the grid as the last. A probe in a real Ghostty window opened on another Space and couldn't be clicked, so this is from the code and Ghostty's padding, not watched in the terminal.
- Folder double clicks: every press toggled a folder, so a double click (or a doubled press) opened and closed it. The second press on the same folder within the double-click time is let go, as VS Code does. `click_on_folder_expands_it` now checks a double click leaves it open and a later click closes it.
- Ignored files: Quick Open and search already left out what `.gitignore` names. The tree now marks those entries, and everything inside an ignored folder, with the same walk filters, and draws them dimmed (`theme.disabled`, icons half into the background). The tour project gained an ignored `target/`.
- Quit button: a red circle with a dark ×, two columns at the far top right, past the pane's own ×, quitting as Ctrl+Q does (asking about unsaved files and what's running; with workspaces, each in turn). The pane's tab row now ends before it. A red circle rather than a second grey ×, so the two don't read as one. Not designed on the canvas first: the user asked for the release without waiting.

Tests changed with the layout: the strip tests count the new two columns; `tabs_in_the_pane_share_the_row_with_the_icons` runs at 180 columns instead of 170, since four agent tabs at their smallest had one spare column there and the quit button takes two. The tour showed the pane's maximize button running under the Comments icon in a 120-column window (it was two columns over before, four now); the pane now goes without it when it doesn't fit, as a double click on a tab maximizes too. A pane too narrow for its tabs still runs the front tab under the icons, as it did before with a fifth tab; letting the front tab shrink broke the rule that it keeps its name, so it was left.

The ui-expert review of the tour pictures led to: one `ignored` colour per theme for both name and icon (the light theme's was 2.8:1 and its icon hardly differed from a plain file's); the quit circle in Catppuccin red with a larger near-black × (white on the light theme's red), the strip's hover chip instead of a darker red, and a hairline setting it apart; the pane's × drawn as the tab's × with the strip's hover chip; maximize kept only with two clear columns before the icons. Its note that the pane's tooltip reads as a sentence, unlike the others, was left: it says what keeps running on purpose.

Unverified: the bar fix in a real Ghostty window; whether a doubled press, not only a double click, caused the folder flicker. Next: the user tries 0.5.3.

The release gate failed twice on `the_vim_keymap_runs_neovim_when_it_is_installed` while containers on the machine used about eight cores: Neovim missed its 2 s first reply three times running, though the test passes alone in a second. Tests now wait 10 s for a Neovim reply; Cellix itself still gives up after 2 s.

## The quit button looks like the other icons (2026-11-06)

The user asked for the close button at the top right to match the other buttons, not be red. It's now drawn as the icons are: grey (`activity.fg`), the text colour and the same `ACTION_PILL` chip on hover, and an × the size of the pane's and a tab's. The thin line before it stays, so it reads as its own group beside the pane's ×. The `quit` and `quit_mark` theme colours are gone. The test now checks it's grey like the icons.

The ui-expert review found the line touching the quit chip and both × left of centre in their chips: the mark grew from its left edge. Every × in the row is now square and centred (`dock::CLOSE_MARK`), the pane's × and its chip sit half a column left, and the line stands half a column before the quit cell, about 8 px clear of each chip. Left as found: a tooltip's shadow reaches up over the hovered chip, for every tooltip in that row; chips are 30 to 34 px wide from rounding.

`click_on_folder_expands_it` drew the screen between its two presses, which took longer than the 400 ms double click at a load near 80, so the second press counted as a new click. It now presses twice and then looks; the single-click case is still covered by the docs folder at its end. `commits_to_push_show_and_push_goes_through` failed in three full runs in a row: one five-second deadline covered the setup's git commands and the push, and under load the setup used it up. The push now gets its own wait. Other deadline tests (PDF zoom and source clicks, pdflatex, disk look, the SSH pane's `pane-send`) failed once each at load 40 to 80 and passed alone, and seven full runs never passed together. Every test wait on background work in `app.rs` (git, a PDF read, a build, the disk) is now one `BACKGROUND` of 30 seconds instead of 5, as `preview_read` already was, and the CLI tests' socket waits 30 instead of 10. `coming_back_to_a_terminal_that_turned_dark_turns_cellix_dark` read a cell's colour the moment the file's text showed, which under load was before the rest of the frame, and panicked on `Default`; it now waits for the colour. What they check is unchanged; only how long they wait for it.

Seen in the real binary through `harness/live/term.py` in a small project: grey at rest, chip and "Quit Cellix (Ctrl+Q)" on hover, beside the open pane with both × the same. The tour's own `13-quit-*` scenes didn't register the mouse on row 0 while OrbStack held the machine at a load near 80 (other mouse scenes worked); their waits are longer now, but they weren't seen working this round.
