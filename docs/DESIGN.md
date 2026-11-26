# Cellix design

Date: 2026-11-01, goal revised 2026-11-07. Progress is in `docs/PROGRESS.md` and
`harness/features.json`.

## Goal

The editor you keep open all day in the terminal you already use, locally or over SSH: VS
Code's layout and keys, what a developer does every day without extensions, and what VS Code
can't do from a terminal. That last part is the reason to choose it:

- Agents built in: the real agent in the side pane, comments it answers, plans and turns to
  review, and sessions that outlive a quit or a dropped connection.
- Documents as pages: Markdown and LaTeX edited as typeset pages, with PDFs beside them.
- Native over SSH: Cellix runs where the code is, with nothing installed on the far side
  and no multiplexer, as light on the machine as a terminal program should be.

It began as a companion for reviewing and correcting agent work. Used standalone, with
workspaces in place of tmux, it became the place the work happens, so parity with VS Code's
everyday features is now part of the goal: the floor, not the point. VS Code's extensions are
not part of it. Most serve a way of working from before agents; languages come from the
registry and language servers, and the rest of the effort goes into the loop of working with
agents: directing them, watching them, reviewing what they did, checking it works, and keeping
what was decided.

## Decisions

| Topic | Decision |
|---|---|
| Rendering | Pixels in the terminal through the Kitty graphics protocol, with a cell-based fallback |
| Rasterizer | Software, on the host (see below) |
| Agents | The real program on a PTY in a side pane, plus `$VISUAL` and thin plugins. Its interface is never rebuilt |
| Editing | Cellix's simple mode by default, optional Vim and Neovim modes |
| Multiplexers | tmux and Zellij are required targets |
| Scope | An everyday editor at VS Code's built-in level, agent-native, with document pages. No extensions; no debugger for now |
| Licence | MIT OR Apache-2.0 |

## Rendering

### Measured wire cost

CPU only on an Apple M5, with zlib level 1 and base64 framing. These are per-frame
encoder numbers. They do not include terminal or network behaviour.

| Case | 1920x1080 | 3840x2160 |
|---|---|---|
| Scroll, full frame | 0.261 MB | 0.728 MB |
| 60 fps scroll, full frames | 125 Mbit/s | 350 Mbit/s |
| Scroll, line tiles | 0.006 MB | 0.013 MB |
| 60 fps scroll, line tiles | 3 Mbit/s | 6 Mbit/s |
| Keystroke, one dirty line | 0.004 MB | 0.010 MB |
| First paint, line tiles | 0.194 MB | 0.540 MB |
| Raster one line | 0.42 ms | 0.79 ms |

Conclusion: never stream frames. Render each line or UI region once as a tile, keep
it in the terminal's image store, and move tiles by reference. Only new content
crosses the wire.

Dragging the side pane's edge is the one common gesture that breaks this: each step lays
both sides out at a new width, so every row is a new tile, about a first paint a step
against a scrolled line's thirtieth of that. With Cellix at the far end of an SSH connection
(`SSH_CONNECTION` or `SSH_TTY` is set, one check everywhere: `pane::over_ssh`) the edge isn't moved while it's dragged: a line in the
accent follows the pointer, stopping where the edge could really go, and the two sides are
laid out once, when it's let go. With no link between, the sides follow the pointer as they
did, and the edge is drawn in the accent while it's held, as VS Code's sash is. Keeping the
resize live over a link would take placing the tiles already sent with a different crop
instead of sending new ones, which hasn't been built or measured. Measured locally with the
counters (release build, a loaded machine): 30 steps of the edge in 0.6 s drew 4 frames,
each a whole layout, 94 rows rasterized and 190 tiles, 1 MB, about 250 ms a present. The
loop already draws only the latest place when frames take longer than steps come, and the
release lands the last one, so herdr's cap of a frame each 33 ms would change nothing here;
what would is not laying the sides out while the edge is held, as over SSH, which changes
what the person sees, so it's left as it is.

### Motion

One mark says something is under way, wherever it stands (`Grid::spinner`): where there are
pictures, a ring at a quarter of its colour's strength with a quarter of it in full colour,
round at its ends, turning; on cells the braille frames `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`. Its colour says what
it means: the accent for working. A turn is ten steps of 90 ms, 0.9 s, and every spinner on
screen stands at the same step, from one clock (`spinner::clock`). A still dot in the same
colour is the same shape with no step (`Grid::dot`), which is what a spinner becomes with
Motion off on Settings' Appearance page (`motion: false`).

The grid says when something on it turns (`Grid::spinning`), and the loop waits no longer
than the next step for input. When that step is all there is to show, nothing is laid out or
drawn again: the frame shown is taken a step on, each spinner's paint and letter
(`Grid::turn_to`), and presented. Anything else that changes draws the whole interface as
before, and the steps carry on from that frame. Nothing turning, the loop waits as it always
has, so a still Cellix sends nothing and wakes for nothing new; a spinner out of view isn't
drawn and asks for nothing. It keeps turning while the terminal says Cellix isn't the window
in front: a window beside the one being typed in is still looked at, and a spinner that stops
there reads as something that hung, for a kilobyte a second.

Ten steps are ten pictures of the row a spinner is on, and they come round: once the row's
ten tiles are in the terminal, a step sends no picture, only that row's placement moved from
one tile to another. Measured with `cargo run -p cellix --example spin` on a window of 150 by
44 at 17 by 37 px cells and rows of 50, a spinner in the side pane's tab row, a row of its
list and the status bar, 11 steps a second:

| Tier | First turn, a step | After, a step | After, a second |
|---|---|---|---|
| Pixels, direct (also over SSH) | 1.8 to 14.3 KB | 83 to 91 B | 1 KB |
| Pixels, tmux's placeholders | 1.7 to 13.8 KB | 163 to 652 B | 2 to 7 KB |
| Cells | 75 B | 75 B | 0.8 KB |
| Inside Claude Code, by value | 11 to 15 KB | the same | 120 to 160 KB |
| Inside Claude Code, by file | 11 MB written | the same | |

So it turns at that rate everywhere but inside Claude Code: over SSH a step costs what a moved
caret does. A row whose other content keeps changing pays a new tile a step until it stops,
since each step is then a picture the terminal hasn't had: on the status bar, a full width
tile of about 14 KB. Inside Claude Code's pane a spinner is the still dot. The mod keeps only
the pictures on screen, so each step there would be its row's picture crossing again, and by
file the whole pane's picture written again; Claude Code has a spinner of its own.

Cellix's own waits turn where VS Code shows progress. The status bar has one item, after the
count of problems: a spinner and what's waited on, the one most likely wanted first: a push or
pull, a PDF building or exporting, an install, a search, a task running, a language server
starting, loading, or doing what it said it began (`$/progress`, and rust-analyzer's own
`serverStatus`), named by its program, "rust-analyzer: indexing". That item is a band of the
bar to itself (`Grid::row_bands`): a step sends its tile, and the cursor moving sends the rest
of the bar and not it, where a step was a new tile of the whole bar while anything else in it
changed. The Search view says "Searching" with a spinner before it under its boxes; Source
Control's button that pushed or pulled turns with its words; a language server being installed
from Settings turns in its row where its button was; a task's tab turns until it ends. A PDF
building or exporting has the status bar's item alone, as VS Code's tasks do, and it names the
file: "Building main.tex", "Exporting notes.pdf". A message saying "Building main.tex…" stood
beside an item saying "Building PDF" until 18 November 2026; the item says it once now.

### The caret blinks

The caret blinks as VS Code's does, the user's pick on 20 November 2026: in files, on pages, in
every text box, and in a terminal pane unless its program asks for a steady cursor. It's half a
second shown and half away, VS Code's editor's figure (`ViewCursors.BLINK_INTERVAL`, 500 ms;
530 ms, first thought of, is Windows' caret time), one phase for everything on screen
(`app/blink.rs`). Any key, paste, press or drag shows it and starts it over, and so does the
caret or a terminal's cursor being drawn anywhere else, so it's solid while typing or moving.
While the terminal says Cellix's window is behind another it stays shown and nothing wakes for
it (VS Code hides an unfocused editor's caret; a window beside the one typed in still says
where it is). Settings › Editor's Cursor blinking (`cursor_blink`, on unless set false, kept in
`~/.config/cellix/settings.json` like the other picks and read again as the window comes
back) turns it off. Inside Claude Code's pane nothing blinks, as nothing turns there: each half
would be its row's picture crossing again.

A terminal pane starts with a blinking block (`dock::term_config`), and its program asks for a
steady cursor as it would in any terminal, with DECSCUSR 2, 4 or 6 or `CSI ? 12 l`, which is
kept; DECSCUSR 0 is back to blinking. Claude Code hides the terminal's cursor and draws its own,
so there's nothing of it to blink.

What it costs. On cells the terminal blinks its own cursor: it's told the cursor's shape and
whether it blinks (DECSCUSR) once, as they change, a bar for a caret and a terminal pane's
program's own shape there, so a blink is no wake and no byte of Cellix's, and leaving gives the
terminal its own cursor back (`CSI 0 SP q`). So a terminal pane's cursor on cells is the
terminal's own cursor at its cell, where it was the cell inverted; one something was drawn over
since, as a tip or a menu, is left as that drew it (`App::finish_caret`). On the pixel tiers the
loop takes the frame shown to the blink's other half, as a spinner's step is taken, with nothing
laid out (`Grid::blink_to`): the caret comes and goes, and a terminal's cursor's cell shows its
own look. A tile is a hash of its row, so once both halves of the row are in the terminal a
blink sends no picture, only the row's placement moved from one tile to the other: 92 bytes in
a window of 100 by 26, about 160 in one of 170 by 46.
The tick is half a second too, so it runs at each half of the blink and not on its own time as
well (`Pace::tick_may_come`), and a window with a caret wakes twice a second as one without
does: run apart, they woke it 4.2 times a second, the tick a moment after each blink.
`a_blinking_caret_costs_a_placement_and_no_wake_of_its_own` holds it to that with the real
binary in the tour's terminal: in front, two presents a second sent, a placement's bytes and no
tile, at no more than 2.05 wakes a second (measured 2.00); behind another window, no byte at
the tick's 2 (1.91).
`harness/x perf` (release builds, a page open in a window of 170 by 46, 2 rounds of 10 s)
against 0.12.0, which has no blink:

| Idle | Window | CPU | Bytes a second | Presents sent a second | Wakes a second | Tiles |
|---|---|---|---|---|---|---|
| 0.12.0 | in front | 0.2 % | 0 | 0 | 2.0 | 0 |
| Blinking | in front | 0.25 % | 328 | 2 | 2.05 | 0 |
| 0.12.0 | behind | 0.2 % | 0 | 0 | 2.0 | 0 |
| Blinking | behind | 0.2 % | 0 | 0 | 1.95 | 0 |

The tour and the harness's other pictures hold the caret still (`CELLIX_CURSOR_BLINK=0`), so a
picture never catches it away; Settings still says Cursor blinking is on there.

### Counting what the loop does

`slow-frames.log` notes frames slow enough to feel and nothing else, which is blind to a busy
core made of many cheap frames. `CELLIX_PROF=1` (or a path) writes a line a second of what the
main loop did: how often it woke, the frames it drew and why (input, a session on screen
printing, the tick, something else coming back, a resize, a glide arriving, sessions out of
sight), the presents and how many of them sent anything, tiles sent, rows rasterized, bytes
written, and the average and longest time in each phase. It's off unless asked, and off it's
an `Option` that's `None`; the bytes are counted always, since that's an addition a write.

`harness/x perf` builds a release binary and runs it, and the last release (Homebrew's, or
`PERF_BASELINE`), in a headless terminal answered as Ghostty answers, with a throwaway home
and project. Three cases: `idle` (a file open, nothing happening), `visible` (a shell in the
side pane printing a line in place 30 times a second) and `hidden` (four shells in tabs
behind the one in front, 60 times a second each). Each warms up for 3 seconds and is measured
for 10, in two rounds of swapped order; a writer that printed much less than it should fails
the run, so back-pressure can't pass for a saving. It runs through the guard, so nothing else
of the guard's runs meanwhile, and isn't part of `check`.

Measured with these counters, in the headless terminal at 170 by 46 on the pixel tier, a
second each (before: the 0.10.2 code, release build; after: the debug build of this work, so
its CPU isn't comparable and isn't given; one round of 6 s each):

| Case | Wakes | Frames drawn | Frames that sent anything | Bytes sent |
|---|---|---|---|---|
| Idle, before | 20 | 2 | 2 | 44 |
| Idle, after | 20 | 0.5, the heartbeat | 0 | 0 |
| Idle, waking only for what's due (0.11.0) | 2, the tick | 0.5, the heartbeat | 0 | 0 |
| Shell on screen at 30 Hz, before | 103 | 31.5 | 31.5 | 265,000 |
| Shell on screen at 30 Hz, after | 34 | 27 | 27 | 268,000 |
| Four tabs behind at 60 Hz, before | 100 | 98 | 98 | 2,200 |
| Four tabs behind at 60 Hz, after | 35 | 2 | 0.2 | 1,000, the tabs' spinners |
| 40 keys typed into a shell, before | 96 | 2.3 a key | 2.3 a key | |
| 40 keys typed into a shell, after | 26 | 2 a key | 1 a key | |

The key's own frame now sends nothing, since nothing on screen changed until the echo, which
draws as soon as it comes. The release builds' CPU against the last release is for
`harness/x perf` when nothing else is building.

### An idle window

An idle Cellix laid the window out and sent the terminal a synchronized update with nothing in
it twice a second, at each tick: a packet each over SSH, waking tmux and the terminal. The
tick now compares marks of what it can change on screen with no key, click or answer saying so
(`App::tick_mark`: the message, files read again or saved, what waits in the inbox, agents found
or gone, how full a session's context is, a picture read, a hover) with those of the frame last
drawn, and draws only when they differ. What the marks can't name, and what tells the time
("2 min ago"), the tick's heartbeat lays out every two seconds, as often as the disk is looked
at (`pace.rs`). And a present with nothing different writes nothing at all, on pixels and on
cells, so a heartbeat over an unchanged window sends no byte. A caret blinking on the pixel
tiers is the one thing an idle window in front sends: its row's placement each half second, at
the tick's wakes ("The caret blinks"); behind another window, nothing.

### Tiles as files, where Ghostty is on this machine

Inline, each new tile is compressed, base64-encoded and written in 4096-byte pieces, and the
terminal undoes all three. Where Cellix and Ghostty are on the same machine, a tile goes as a
temporary file instead (the Kitty protocol's `t=t`, as herdr does for local Ghostty): Cellix
writes the tile's raw pixels to a file and writes only its name, and Ghostty reads the file
and deletes it. A tile costs a couple of hundred bytes on the wire whatever its size, and
neither side compresses anything, since the file is read from memory. That's what makes the
frames full of new tiles cheap: the first paint, a resize, dragging the side pane's edge here,
a theme change. In the headless terminal a first paint was 233 KB inline and 13 KB as files
at 1080p, and 710 KB and 25 KB at 4K, the pictures the same byte for byte; PROGRESS's entry
has the rest.

**When.** Only where `TERM_PROGRAM` is `ghostty`, on Unix, with none of `SSH_CONNECTION`,
`SSH_CLIENT`, `SSH_TTY`, `TMUX`, `STY`, `ZELLIJ`, `CLAUDECODE` or `CELLIX_MOD` set (set at
all, even empty), and only on the direct tier: never over SSH, inside tmux or Zellij, in
Claude Code's pane, on placeholders or on cells. Then one more question goes with the start-up
ones, before device attributes: a 1 by 1 picture in a file, queried (`a=q,t=t`). Tiles go as
files only if the terminal answers OK and the file is gone (within 100 ms of the answer, which
Ghostty doesn't need). Otherwise they go inline for the run. Later, a file that can't be
written, or one the terminal hasn't taken within 10 seconds, turns files off for the rest of
the run; a tile past a cap goes inline. `CELLIX_TILE_FILES=0` turns it off.

**Where, and why it's safe.** The files hold the screen's pixels, the code under review, so
nobody but the user may read them, and nothing may be tricked into writing somewhere else.

- The temp folder is the user's own: `TMPDIR` where that's a folder of theirs that nobody
  else can write to, as macOS gives each user under `/var/folders`, or else `/tmp`. Ghostty
  takes a temporary file only from `/tmp`, `/dev/shm`, `/private/tmp` on macOS and its own
  `TMPDIR`, judged by the path of the file it opened, and only with `tty-graphics-protocol` in
  that path. Started inside Ghostty, Cellix has its `TMPDIR`. One that differs fails the probe,
  and tiles stay inline.
- In it, `cellix-tiles-<uid>` is made 0700 if it isn't there, then opened with `O_NOFOLLOW`
  and `O_DIRECTORY`, and used only if it is a folder of the effective user's with mode exactly
  0700. A link planted in its place, or a folder someone else made, means tiles go inline,
  never a write elsewhere. In `/tmp` its sticky bit stops anyone else renaming or removing it.
- Each run makes a folder of its own in that, `session-` and 128 random bits, freshly (making
  one fails where the name is taken, and another name is tried), 0700, checked the same way,
  and holds an `flock` on it while it runs.
- Each tile is `tty-graphics-protocol-` and 64 random bits, created with `O_EXCL` and
  `O_NOFOLLOW`, 0600, and written whole and closed before the frame that names it is written.
  Nothing is overwritten, and nothing is written outside the run's folder. Cellix never reads
  one back.
- Caps: at most 1,024 files and 64 MiB waiting for the terminal, and 16 MiB a tile. herdr's
  are 8 files, 64 MiB and 16 MiB for whole images; Cellix sends a tile a row, a few hundred
  for a first paint at 4K, which is about 25 MB of raw pixels. Past a cap, a tile goes inline.
  At most 16 runs' folders, live or left behind, or tiles go inline, so runs that crash one
  after another can't fill the disk.
- A file counts as taken only when it isn't there (an I/O or permission error doesn't count),
  and they're looked at oldest first, since the terminal reads them in the order it's told.
- Ghostty deletes each file it reads. At the end of a run Cellix removes what's left and its
  folder, never recursively; a stop the loop can't see to clears the folder as it clears a
  workspace's socket. At the next start, the folders of runs that ended without clearing up
  are cleared: those no run holds the lock of and that are a minute old (a terminal may still
  be reading what a run that just ended left), and only if all they hold are the user's plain
  files with a tile's name and one link. Anything else, and the folder is left as it is.

### Output out of sight

A session in a tab behind another, or in a workspace not in front, prints as much as the one
on screen, and every look at the sessions that found any of it drew a whole frame and kept the
loop at its quick pace: four tabs printing behind the one in front drew a hundred frames a
second that showed nothing new. What a session out of sight prints is still parsed as it comes,
but it draws nothing then (`Dock::poll` says what changed on screen apart from what changed
behind it, and `Workspaces::poll` counts everything in a workspace not in front as behind).
What's drawn from it, its tab's mark and name and the workspace's tile, is brought up to date
at the next tick, so it's at most half a second behind; a session ending, which closes its tab,
draws at once.

### What wakes the loop

The loop waited for keys only, so a session's output was seen at the wait's end: every 8 ms
while anything was lively, 40 ms after that, which kept it waking a hundred times a second
with a shell at its prompt, and a key's echo often came a frame after the key's own. Where
the terminal is standard input (Unix), the loop waits on it and on a pipe of its own, which a
session's reader pokes as output comes (`wake.rs`): output on screen is drawn as it comes,
at most once every 16 ms, except a key's echo, which for 32 ms after the key draws at once (up
to 16 frames). A session out of sight doesn't poke unless what it has printed is piling up.
A reader's queue holds 64 reads, about half a megabyte; a program printing faster than that
is taken in waits, rather than Cellix's memory growing.

Nothing wakes the loop on a timer of its own: an idle window wakes only when something is due,
as herdr has it. It used to wake every 50 ms in case a language server or a thread had
answered, about twenty times a second with nothing to do. Now everything that answers pokes the
same pipe: every thread that hands the loop a result (`wake::channel`, whose sender pokes as it
sends and as it goes), the language servers' and highlighting's threads (`cellix_lang::wake_with`),
the agents' socket and the IDE's (`cellix_agent::wake_with`), a session's end, a stop, and a
resize, whose signal pokes it beside crossterm's own (`signal-hook-registry`). The look at the
disk every two seconds pokes only when it saw something the last look didn't. The wait then
ends at the soonest of the tick, a spinner's step, the caret's next half on the pixel tiers
(with which the tick then comes, "The caret blinks"), a frame of output waiting, a program's
synchronized update running out, and the terminal asked its cell's size once the window
settles, rounded up to the millisecond so it never ends just before what it waits for. What is
timed from a moment just past and checked as the loop goes round (a question for an agent
once it's been quiet, a spell check once typing pauses, the hint over a wide page, an agent's
call under way, a notification waiting for its gap) has `App::soon` say so, and the loop looks
every 50 ms while it does. Idle, it wakes twice a second, for the tick, where it woke 21 times.
A session out of sight is taken in at those wakes, or sooner once it has half filled its room. On Windows the
console's input isn't a descriptor to wait on beside a pipe, so the loop there still looks at
the sessions every 8 ms while they're lively and 40 ms after.

### No GPU renderer

Output would have to be read back to the CPU before it could be sent to the terminal, one
line rasterizes in under a millisecond on the CPU, and a remote host often has no GPU at
all. The on-screen GPU work is done by the user's terminal in every case. Looks come from
the paint layer, not from where pixels are computed.

### Observed behaviour by environment

Static page of the real editor, checked by screenshot in Ghostty 1.4.0 on macOS with
`harness/visual.sh`, 2026-11-01.

| Environment | Method in use | Result |
|---|---|---|
| Ghostty alone | Direct placement | Works |
| tmux 3.8, `allow-passthrough on` | Unicode placeholders | Works, no visible seams |
| Zellij 0.45.1 | Direct placement | Works |

In the spike, direct placement failed under tmux (all tiles on one row) and placeholders
rendered nothing under Zellij, which is why the method is chosen per environment:

| Tier | Used when | Method |
|---|---|---|
| Pixels, direct | Terminal or Zellij answers the Kitty graphics query | `a=p` placements |
| Pixels, placeholder | tmux with `allow-passthrough on` and a Kitty-graphics client | Passthrough transmit, U+10EEEE cells |
| Cells | Anything else, or no known cell size | Styled text cells |

Cells are a full tier, not an error state: every feature must work there. This is
what guarantees tmux and Zellij always work. Pixels are the upgrade.

Detection sends `CSI 16t`, `a=q` and device attributes, and checks `$TMUX` and `$ZELLIJ`. In
Ghostty on this machine it also asks whether a tile can go as a file (see Tiles as files). The background colour
(`OSC 11`) decides light or dark, and is asked again each time the window regains focus, since a
terminal follows the system's appearance. tmux answers that question from what it learned when
the client attached and keeps the terminal's own answer to a passthrough question to itself, but
the passthrough question is how it learns the new colour, so inside tmux Cellix asks past it
first, waits for the cell size answered after it, then asks tmux.
tmux swallows the replies, so under tmux Cellix asks tmux for the client terminal name and
`allow-passthrough`. `CELLIX_TIER` can force a tier. `CSI 16t` is preferred for cell size
because the window size call was not reliable under Zellij at startup. Window size is
polled as well as signalled, because a resize during font loading is otherwise lost.

The answers come on the input the keys come on, and one that comes after its wait (250 ms,
three times that past tmux) has run out, over a slow link or on a loaded machine, is read by
the loop. crossterm makes keys of what it doesn't know, so a late background answer was typed
into the open file as `11;rgb:1010/1111/1414`. Everything the loop reads goes through
`cellix_term::Replies` first, before the clipboard's reader, which knows the shape of every
answer the questions bring (the background's, the cell size, the modes' reports, the graphics',
device attributes, past tmux or not) and drops it wherever it arrives: whole, in pieces across
reads, after a lone Esc where a read ended on an answer's ESC, or as the rest of one whose start
a wait read, which the wait says (`replies::owed`). What may start an answer is held until it's
clear, and goes on as it was if it isn't one: for ten seconds after a wait runs out an Esc,
Alt+] or Alt+_ alone waits up to 100 ms for an answer's rest, and otherwise not at all, except
where paste events are on, where it always waited that long for a clipboard message's rest. A late
background answer still sets the colour, and a wait takes the last colour and cell size it read,
since one before may be an earlier question's.

### Known rendering problems

- The placeholder seams from the spike are gone at the screenshot's zoom. The spike
  guessed the cell size and Cellix rasterizes at the exact size, which is the likely
  cause, but this has not been checked at other font sizes.
- Pixel tiers draw with the first system monospace font, not the terminal's own font,
  so text changes appearance between tiers. The font should be a setting.
- A tmux tile stream costs more than direct placement, because each cell is written
  as a placeholder. This has not been measured.

## Not yet proven

These decide whether pixels stay the default:

1. Live scrolling in Ghostty, tmux and Zellij. Real use in Ghostty showed text overlapping,
   because a placement belongs to its image and a new image doesn't replace it. That is
   fixed and modelled in a test, but smoothness and the fix itself need checking live.
2. Pixel tiers through tmux and Zellij with split panes, pane switching, resizing and
   detach. `harness/x mux` covers tmux splits, resizes and window switches, but with no
   client attached, so on the cell tier. Zellij has no automated test yet.
3. A real SSH session on a slow link.
4. Other terminals: Kitty, WezTerm, iTerm2, and whether Zellij behaves the same on them.
5. Pixel mouse reporting (mode 1016) through Zellij.
6. Rows of Cellix's own, taller than the terminal's line, which the look picked on 16 November
   2026 rests on: a row of about 26 pt for 13 pt words, whatever the terminal's line is, with
   no setting of the terminal's touched. Settings' Row height chooses it (see Row height),
   on the direct tier where the terminal reports the mouse in pixels, and nowhere else: under
   tmux, in Claude Code's pane and on cells a row stays a line. The grid has as
   many rows as fit the window's pixels. A row's tile is placed at its own size on the line
   its row starts in, `Y=` pixels into it, and the last row's runs on over what the lines
   leave under it. Words, icons, the caret and any shape shorter than a row keep the size
   they have on the terminal's own line and stand in the middle of the taller row, so the row
   is air round them; a terminal's box-drawing is drawn to the row's whole height, so its
   lines still meet. A press is so many rows down by its pixels. Nothing glides: the glide
   counts in lines. Proven by tests, by the real binary in the headless terminal (presses,
   typing, the wheel) and by one capture of a real Ghostty. Not yet: a person's hand on it in
   Ghostty and kitty, and over SSH; the glide; pictures and PDF pages on a taller row; and
   every one-row rule in `measures.rs` looked at again on one.

If 1 or 2 fail, cells become the default in that environment.

## Paint layer

Cells alone read as an ASCII drawing, even as pixels. So the grid stays the source of truth
for layout, hit-testing and the cell tier, and a list of paint items sits above it for the
pixel tiers: proportional text, rounded panels, hairlines, shadows, icons and a thin caret.
Paint positions are in 64ths of a cell, so they can sit between cells. A row's tile hashes
the paint that touches it, so caching and reuse work as before. Cell text marked `painted`
is skipped on pixel tiers and kept for the cell tier and tests.

Fonts are embedded (Inter, JetBrains Mono, and Twemoji for emoji), because pixels are drawn
on the remote host over SSH and servers rarely have fonts.
Inter is set with its contextual alternates off, so what it shows of code and of what people
and agents wrote is quoted as written: they drew "->" as "→", and a hunk's header and Search's
results misquoted a function's signature (0.11.0's final review). A row of a list too long for
it, Review's Compare-with or a Search result, is cut with a "…" clear of its band, never
through a letter at the list's edge.

Text is measured with the same fonts the rasterizer draws, through the `Measure` trait, so
carets and wrapping use real glyph positions. The cell tier measures one column per
character. Where the terminal offers mode 1016 the mouse reports pixels, because a cell
can hold two thin letters.

An emoji in painted text is drawn in the machine's emoji font, in its own colours: a line's
mask keeps the colour of each pixel that has one (told from text by drawing the line in white
and again in black: what changes is text). It's set as a run of its own in that font, because
Inter holds a plain heart and a plain smiling face and would draw those. The measure sets it in
the same font, which it finds once for the process and loads by itself beside the built-in ones,
so a caret after an emoji is where the emoji ends. The cell tier measures by what reads as one
letter, so an emoji of several code points is its two columns once.

Which font that is: the machine's own (Apple Color Emoji, Noto Color Emoji, Segoe UI Emoji), so
emoji look as they do in its other programs, when it has one this rasterizer draws in colour,
which is tried, since a newer Noto is in a format it can't read; and otherwise Twemoji, which is
in the binary (Mozilla's COLR build, 1.5 MB, vector, Emoji 14.0), so a server reached over SSH
shows emoji too. `CELLIX_EMOJI=built-in` keeps to Twemoji everywhere. Noto's bitmap font draws
as well and is current, but is 10.7 MB, a third of the download; the user chose Twemoji.

Paint can be kept to a band of rows (`Grid::keep_to_rows`): on any other row it isn't drawn, and
a row's tile doesn't hash it. A list scrolled in a pane paints its cards whole, and the part of
one past the pane's foot was landing on the status bar. The Comments pane keeps all it paints to
its own rows; rows are what tiles are made of, so a band of rows is an exact cut.

Tiles are whole rows, so one region can't scroll by itself. For a wheel notch on the direct
tier, a layer of region tiles goes above the row tiles, moves by pixel offsets, and is
removed in the same synchronized frame as the final page. Nothing in the row pipeline
changes, and tiles move by reference, so a frame is only placements. tmux can't place
between cells, so it keeps jumping by lines.

## Pages

A page keeps to a reading width: its text is set to at most 88 columns, about 140 characters
a line in the interface font and 88 on cells, and in a pane wider than that it stands in the
middle. Tables wrap their cells to that width and blocks of code end with it, so the page has
one right edge; what's still wider (a table with too many columns, a long line of code) moves
sideways inside the page, and a letter that would be cut in half at its edge is left out, as are
all of a table's cell's words once either edge cuts them, its box and fill kept: "col06" showed
as "l06" at the left, and "cc" and "1.1" at the right (0.12.0's release review). A misspelt word in
a table's cell has its dots up under its words' tails, inside the cell, where at the row's foot
they lay on the cell's line.
The middle is that of the columns the page has: the pane's, less its minimap's when one is
showing, so a long page isn't pushed against its minimap. A page is laid out before that's
decided, so its first frame is where it stays. A pane no wider than the page sets it as
before, from its own margin, with blocks running into the right margin. A sheet of data (CSV, TSV) keeps the whole pane,
since it's read across. `reading_width: false`, or Reading width under Settings, Editor, lets
every page run the width of its pane, as all pages did from 1 to 10 November 2026 by the
user's choice then; the user chose the reading width after a day in a wide window. A line
break inside a paragraph shows as a space that keeps the newline's source position, so a
hard-wrapped file fills the page and saves unchanged. On a page, arrows and Home and End move by what's shown.

A page selects as any document does, with one selection, the editor's: a drag, a double click for
a word, a third for the line, Shift with the arrows, and a click that sets where a selection starts
and a Shift+click that ends it there, across headings, lists, quotes, tables, code and formulas,
however far the page has been scrolled in between (the user's ask, 18 November 2026: "scan it
through"). A drag covers more than the screen too ("or selecting text with scroll support"): held
past the text's top or bottom edge, onto the tabs or the bar at the foot, the text scrolls that way
and the selection takes in what comes onto the edge row under the pointer, at 8 rows a second just
past the edge and 16 more for each row further, as browsers grow it with the distance, the end
kept in view where the edge row is empty and the text after it is further on; it stops when the
pointer comes back over the text or lets go. Only while it's held there does `App::soon`
have the loop look every 50 ms, so nothing is timed otherwise. The wheel turned with the button
held scrolls, and the selection follows to what's under the pointer then. Until then a drag past
the foot scrolled only as far as the pointer moved, one past the top not at all, and the wheel left
the selection where it was. Another Shift+click moves the end and keeps the start, and one after a drag or a double
click keeps that selection's start. A Shift+click is about the selection wherever it lands, so it
ticks no box, folds nothing, opens no included file and isn't counted towards a double click; a
plain click on those still does what it did. An end on a formula or a picture, by a drag or a
Shift+click, takes in the whole of it, so it stays drawn and selected rather than opening to half its
source; one already open is text like any other. One inside a selection reads as taken in: a
formula's ink stands on the selection's colour, with half a column and 7 px of it round the ink as
a word's letters have, and a picture of its own colours lies under a see-through wash of it, as a
browser shows one. A selection is drawn as a browser or VS Code's Markdown preview draws one, a
shape that reads as one: each line of it is one stretch the height of its row, so a paragraph's
lines touch, painted over inline code's and a highlighter's grounds and under the words; a list's
bullet or number, a task's box, a section's number and a citation's brackets are inside it where
the selection runs through them, and an equation's number goes with its picture; a table's cells
taken whole are filled from line to line, under the table's lines, which still say it's a table.
Until 0.11.0's review it was a band for each run of one look, 38 px of a 50 px row, so it had a
seam at every change of look, holes where code and highlights kept their grounds, a gap under every
line, the markers and numbers left out and a table selected word by word. The status bar counts the
words selected in a paper as it counts the paper's, without its commands. The selection gets Comment
and Ask Agent as a dragged one does, by the rule below, and its comment quotes its first line and
hears all of it. Code in the source view, notebooks, data, plans and transcripts are the same
editor, so they select the same way. Ghostty and xterm keep a Shift+click for their own selection
unless the program asks for it, so Cellix asks as it starts (XTSHIFTESCAPE, `CSI > 1 s`) and hands
it back as it ends; until it did, a Shift+click on a page selected the terminal's cells instead.
Inside tmux, the outer terminal isn't asked, and Claude Code's pane passes no Shift with a click.

### Companion

A press on words a comment is about lights that comment's card in the Comments pane, with
the accent's edge, and scrolls the pane to it; a press on other words puts the light out. A
pane open on another tab turns to Comments for it, the tab it was on staying in the row (the
user's ask, 14 November 2026: until then the card was lit behind whatever was in front, where
nobody saw it). That's only for a press that puts the caret in those words, not one on the
scrollbar or a checkbox while the caret is already there. A pane that's shut stays shut, since
the pane holds what was opened and a press in the text isn't a press on the pane. So with the pane
shut, resting the pointer on commented words shows their comment on a card, made as a menu is
(raised, half a row of air over its first words and under its last, two columns in): who made it
and its kind, quietly; its words, three rows at most; the last answer, its verdict on a chip and
its author in bold, as the pane's card has them, two rows at most; and under a rule, how many answers and "Open in
Comments" in the accent. A press on the card opens the pane on that comment as a click on its card
in the pane does: its words selected, its card lit and in view, the keyboard in the file. The card
stands on whole rows of its own under the words, its words starting where theirs do, or over them
where there's no room under; never on them. Where a row has air the panel stands down into the
next row's air over its words, up to 8 px, clear of the words' tint, and what whole rows leave over
goes to the room round its rule, as a menu's goes to its rules. It goes
when the pointer leaves the words and the card, and at a press anywhere else, the wheel or a key.
It's for words a comment tints, which pages show; code marks a commented line in its gutter
instead, and has no card (`app/glance.rs`). Next Comment
and Previous Comment, in the palette with no key of their own as VS Code's Next Comment Thread
has none, go through the file's comments in the order their words stand in, round at the ends:
each opens the pane on its comment, selects its words and lights its card, and the status bar
says "Comment 2 of 7". A press on a line's ◆, in the gutter or a page's margin, does the same
for that line's comment (presses there only folded). Check My Review is in the palette too. It goes the other way too: a click on a card goes to its
words, selects them so they're plain to see, lights the card, and leaves the keyboard in the
file; the buttons that offer to comment on a selection stay away from those words. The pane lists in the document's order, or by which comment was made last or first
(a comment's number counts up as they're made), chosen from a
button under the kinds, which names the order in use beside a chevron and opens a list that marks
it. With no file in view a heading still
goes wherever the file changes, so by recency a file can have more than one.

Resolved comments are folded away under a button that stands after those still open: a chevron
and "Show N resolved". Pressed, it stays where it is, its chevron turns down, and the resolved
ones unfold under it in the pane's order, so the same press in the same place folds them away.
Until 14 November 2026 they were mixed in among the open ones and the button ended the list, so
with eighty resolved it was eighty cards from where it had been pressed, and it was loose words
that read as a third line of the hint over it. The user asked for both controls as buttons with
chevrons. They're the quiet chip a card's own buttons are, on the pane's ground, starting at the
pane's text edge where the summary and the kinds start, with their words a column inside, on the
column the kinds' words and a card's are on (`threads::control`). The order's chevron comes after
its words and is the small one the branch's and the heading's menus have, since it opens a list;
the other's comes before and is a folder's, right while they're folded and down while they show,
since it unfolds what's under it. That one says "Hide N resolved" unfolded and is as wide as the
wider of the two, so nothing of it moves when it's pressed. Under the kinds or a card a chip is
painted two fifths of a row down its row, into the empty row after it: 21 px under the kinds and
18 over the first card, 30 under a card and 18 over the one it heads, where a row of its own each
side left it adrift in three. It's pressed where it's painted, by the row's fraction the terminal
reports (`threads::hit`), and the order's list drops from the first whole row under it. Under the
words that say no comment is open it keeps its row, an empty one each side.

A review has an output. Comments are kept in `.cellix/threads.json`, out of git, so a finished
review, a referee's report say, went nowhere. Comments: Copy Review as Markdown puts it on the
clipboard the way every copy goes (OSC 52), and Save Review as Markdown… asks for a name in the
project, offering `review.md` or the next one free (`review-2.md`), asks before it replaces a
file that's there, writes it and opens it as a page. A right click on the Comments tab has both:
the pane has no header menu of its own, and the list its order button drops is only orders. The
order picked there (Document order, Newest first, Oldest first) is kept as `comment_sort` in the
user's own `~/.config/cellix/settings.json`, so it holds for every Cellix on the machine, and a
workspace's `.cellix/settings.json` can still set its own. A Cellix already running reads it again
as the menu opens, as the list is shown and as the terminal comes back to it; there's no watch on
the file. Until 19 November 2026 it was the session's only (the user: "it should be persistent for
all cellix sessions on the machine"). Tests read nobody's own settings file. The
review is every comment the reviewer kept, open and resolved: a file at a time by name, as the
pane lists them with no file in view, a terminal's output last; in each file a kind at a time,
in the strip's order, those with no kind last under Other; and down the document within a kind.
An agent's suggestion goes in once it's kept, which makes it the reviewer's; one dismissed or not
yet decided doesn't. A short head says how many comments, on how many files, how many resolved,
and how many of each kind. Each comment is headed by its line and whether it's open or resolved, quotes
its words (the whole selection) as a block quote, and has the reviewer's words and every answer
under them, named, an agent's verdict first as its card puts it, "Agrees · Claude". Its heading
ends with when it was made, as a date and time in UTC ("Line 4 · Open · 2025-11-18 13:00 UTC"),
since a review is read anywhere; one kept from before comments said when says nothing of it.

A comment and each answer say how long ago they were written, after who wrote them: "You ·
5 min", "Claude · 2 h" (`at`, seconds since 1970, kept with each; a comment's number had been the
only record of when, and one kept from before says nothing). A suggestion has Reply between Keep
and Dismiss, so the agent can be asked about it without its being kept; it's waited on as the
reviewer's own comment is, and the agent hears it's its own suggestion. A suggestion's card stands
on the ground every card has, with a dashed edge in a quiet grey until it's kept: the accent's
tint and edge are the lit card's alone (a suggestion was a shade of the accent, which under a lit
card read as the same card). Words a comment was made on that are gone, "Words gone" and "Removed
in version 2", are in the removed colour, as a diff's removed lines are, not the colour of what
needs you. A press on a kind's pill
lists only that kind's comments, the pill in the accent's colours with a × after its count, and
another press lists them all; with none of that kind the pane says so and how to list them all.
While a kind is listed alone the head says so, "Evidence · 1 of 4", in place of how many are
whose (it said "3 of yours · 1 suggested" over the one card). Every other pill is an outline, its
kind in the words' colour where there are comments of it and quiet where there are none, its
count quiet: filled grey, the kinds with comments read as switched on, which only the picked one
is. The pills count the
reviewer's own comments: a suggestion not yet kept covers nothing, and was counted. With no
comment open, the pane says to select text and press Comment, the button a selection shows,
which names the keys as they're set; it named only Alt+C. With suggestions in the text it names
them first, "3 suggestions in the text: Next goes to each.", since "3 suggestions" over "No
comments yet." read as the pane saying two things. A list that runs past the pane has a thumb at
its right, in the air between the cards and the sheet's edge, as the file has; and the order's
and the resolved comments' buttons, which stand part of a row down theirs, aren't painted or
pressed on the pane's last row, where its foot cut them in half, until a scroll brings them up.

An agent that has done what a comment asks says so with `cellix reply <id> --done "what it
did"`, a reply that starts "Addressed:". Its card says "Addressed by Claude" in a green pill, and
its buttons are Resolve and Not Done: the reviewer settles it, never the agent. The answer under it
has no chip of its own while the pill says so (the card said "Addressed" twice); an earlier one, or
one on a resolved card, has the chip. Not Done begins a reply "Not done: " for the reviewer to
finish, which goes back to the agent as any reply does. "Resolve N addressed", a quiet button at
the end of the order's row, settles every one of those listed. The agents' skills, and how every
comment is put to an agent, say so.

The same thing has the same words wherever it's shown. A button says what the palette calls the
same thing, in VS Code's Title Case: Ask Agent and Check My Review on the cards and the pane's head,
as in the palette, Problems and a selection's buttons, where a card had said "Ask agent" beside its
own hint's "Ask Agent"; Not Done and Not This too. A button with a count in it says it as words
("Show 2 resolved"). An agent's verdict reads as the pane's card has it on the card the pointer
brings up over the page too: its word in its colour, then who ("Agrees Claude: …"), where that card
said "Claude agrees:"; and as it shows the last answer, its foot counts the others ("1 more reply ·
Open in Comments"), where it said "1 reply" over the one reply it showed.

The pill on a comment whose words were reworded since, "Changed since your comment ›", opens
what changed in the quote's place, on its bar: the words commented on and the words there now as
one run, a word at a time, what went in the removed colour and struck through, what came in
green, what stayed as the quote's words are (`threads::word_changes`, from `similar`'s word
diff), and the pill's chevron turns down. Another press on the pill closes it. (It opened as a row
under the quote, which said most of the quote's words again.) A plan's "Changed in version N ›" goes on opening the plan's versions
side by side, since a plan has each version whole; a document has only the words commented on.

Where the pane is small, in a window as narrow as Claude Code docks Cellix in or stacked under the
file, a card is short until it's lit: who and when, its pills, what it's about, and two rows of
its words, the second ending "…" where there's more. Its last row says how many answers it has
and what it waits on ("2 replies · Waiting for Claude Code…"), and while the pointer is on the
card its buttons stand there instead, so nothing moves as the pointer comes and goes; a card with
nothing to say there shows its buttons, so no row stands empty (a kept row with nothing in it read
as slack in the pictures). Lit, by a click on it or on its words in the file, it's whole. A wide pane keeps
every card whole, which read better there in the tour's pictures.

A comment's kind is picked as it's written: under the hint stand the five kinds, a chip each,
outlined and quiet as a coverage pill with none is, the one picked in the accent's colours as a
kind listed alone is, and a press on it again takes it back. Changing a comment starts from its
kind. A reply has none, since it's about its comment. Until 18 November 2026 a kind came only from
an agent's verdict.

A web address in a comment's words or a reply's (`http://` or `https://`, up to the next space,
without the full stop or bracket that ends its sentence) is drawn in the accent and underlined,
on every row it wraps onto, and a click on it opens it the way Open in Browser opens a page: the
side pane's browser where one is set up, the system's otherwise. A click anywhere else on the
comment is still the comment's.

Each card says its comment's number, last in its header and muted, as VS Code's comment says
when it was made: "You · Clarity · 5 min · #33". The comment glance card says it after who made
it, and the review's Markdown at the end of each comment's heading. Agents are told "Comment 33
on …" and answer "#33" or "comment 33", which no card showed (the user, 20 November 2026:
"there are references like comment #33, etc., but since comment boxes are not showing them it's
impossible to find that #33"). Such a mention, "#33", "comment 33" or "Comment #33", in a
comment's words, a reply or an agent's answer, is a link, drawn as a web address is; a "#" inside
a word or an address isn't one. A click on it opens the Comments pane on that comment, lit and in
view, its words selected in the file, as a press on a line's ◆ does; a kind listed alone gives
way, a resolved comment is listed with the resolved ones and the bar says it's resolved, and one
that isn't in the project says so. A right click on it has Go to Comment #33. In a terminal's
output in the side pane, Ctrl+click on a mention goes there too, and a right click has the same
row. The Comments pane has no filter of words to find "#33" by, so none was added.

A right click on a link, wherever one can be clicked, opens a menu with Open Link and Copy Link
(decided 18 November 2026: the user found a link opened in the terminal's browser hard to
share). It drops from under the pointer's row, at the pointer's column, so the link it's about
stays in view: opened on the pointer's own row, as a code menu is, it hid all of the address but
its first letters. Over a comment's card or peek it's a menu's colour as every menu is, its line
and shadow setting it off the card. Open Link does what a click on it does, or Ctrl+click where a click puts the
caret; Copy Link puts its address on the clipboard the way every copy goes, by OSC 52 through
the terminal, so it reaches the person's own clipboard over SSH and in tmux, and says "Copied the
link". The address is where the link goes: a Markdown link's target, `\href`'s and `\url`'s
argument with LaTeX's escapes taken out, never the words it's shown as. A link to a file has
Open File and Copy Path instead, as a `file:line` in a terminal's output, a Markdown link to a
file beside the page, a paper's `\input` row and a comment's picture shown as its name (Open
Picture); a link to a place on the page itself, `#section`, has neither. The places are a comment's
words and its replies in the Comments pane, a file's text, rendered or as source (transcripts
are pages), and a terminal's output in the side pane and in peek, where a web address now opens
on Ctrl+click too, found whole where it wrapped onto the rows under it. Where a right click
already has a menu, as on a page or in code, the link's two rows stand at its top, a rule under
them; in a terminal, where it had none, a right click off a link still opens none. On a comment's
card it opens the card's own menu, the link's rows over it where there's a link.
No key opens it: Cellix's context menus have none (no Shift+F10), and Open in Browser in the
palette is the keyboard's way to the link at the caret. The Task page, Settings, Review's comment
cards and the inbox show no links to click, so they have none.

A card's words and its answers' can be selected with the mouse as a page's are (the user, 20
November 2026: "I cannot do copy paste from comments, which is a crucial feature"): a press on
them goes to the comment's words in the file as it always did and begins a selection, a drag
carries it on, a double click selects a word, a triple click the line of the comment or answer
it's on, every row it wraps into, and Shift+click reaches from where it began. It's tinted in the
selection's colour taken toward the accent until it stands 1.8 to 1 from the card under it, about
as far as a page's stands from the sheet (`Theme::selection_on`): in the selection's own colour it
was 1.27 to 1 from a lit card and could hardly be seen. The words on it are deepened where they'd
read under 4.5 to 1 there, as the light theme's are. The copy key, Command::Copy as Cmd+C or Ctrl+Shift+C reach it,
copies it, the way every copy goes, by OSC 52 through the terminal: as written, with the new lines
it had where rows wrapped and a new line between a comment's words and an answer's. Any other key,
or a press off the cards, lets it go, and the copy key is the file's again. A right click on a
card has Copy where its words are selected, Copy Comment (the comment's own words, as written) and
Copy Quote (the words it's about, as its agent is told them), with a link's rows over those. What's
pasted into a comment being written goes in whole, its lines and all, as the terminal pastes it
or from Cmd+V.

A comment takes emoji three ways, all of which leave the emoji itself in its words, so an agent
reads it as written. A name between colons, GitHub's names, becomes its emoji as its second colon
is typed (`:+1:`, `:100:`), unless a letter, a digit or another colon comes right before it, so
`a::x::b` and 10:30:00 stay as typed. A name half typed (two letters or more) is offered under the
words in place of the hint, the nearest first: Tab takes that one and a click any. Under the words
stand the eight last used, the usual eight until then, each a click, kept between runs with the
project's other state; after them a face opens every emoji in a grid under a search field, by kind
under the most used, where the arrows move, Enter or a click takes one and typing looks through
names and GitHub's names both; the grid is as tall as what's found, up to nine rows, where it stood
about 450 px empty under one row, and opened over its place it keeps its foot there. What's offered is what the emoji font in the binary holds whole (Emoji 14.0, asked of the font
itself once), so no machine is offered a box; one typed by name is taken whatever its age. A name
an agent spells out in an answer shows as its emoji. Should no font draw emoji in colour, none
of this shows and names stay as typed.

A picture is in a comment as a word of it: its path from the project's root, which is what an
agent reads and can open, so nothing about a comment's shape changed. It gets there while the
comment is written: pasted through the terminal, dropped as a file (which arrives as its path),
with Paste's key when the clipboard holds a picture and no words, or from the picture button under
the words, which offers the project's pictures and the clipboard's. One from outside the project,
or with a space in its path, is copied in under `.cellix/pasted`, out of git, with its spaces
closed up; one of the project's own is named where it is. A word names a picture when it ends as
a picture's file does (png, jpg, jpeg, gif, webp, svg) and isn't a web address. Where the screen
shows pictures the card takes the name out of the words and stands the picture under them, at its
own size up to the card's width and eight rows, with a line round it, on an answer as on the
comment, and under the words being written too; a click on it opens it in the Preview. It's read
small once and again when its file changes, so a picture an agent makes after naming it shows
when it's there. Where the screen can't, or the file is missing, the name stays in the words as
a link that opens it.

A comment being written stands in a box, as a field does, on the card's columns (it stood 8 px
from the card's left edge and 17 from its right), its words a column inside it: its words and,
inside the box under them, the eight emoji and the two buttons for the rest and for a picture, each a glyph in the
middle of a square chip that shows while it's pointed at. A row is kept between the words and
the tools, with the words a little down into it and the tools a little up, so both stand clear
of the box's lines, and the list of every emoji hangs from its bottom line. Under the box is the hint, or the emoji
offered for a name (the first on the selection's chip, with Tab named at the row's end), and under
that two buttons: the one that sends it, filled and named for what sending does (Comment, Reply,
Save, Ask Agent), and Cancel. They do what Enter and Esc do. While a comment is answered or
changed they stand in place of its own buttons, since one thing is being done to it. The filled
button is the pane's one: what's being written is what's under way.
The list's head, what's yours and what's suggested, Check My Review, the kinds and the order,
stands on the cards' edges: from where a card starts to where it ends, where there are pictures
(it stood 8 px inside them on the left and 17 on the right). An agent's answer stands on the
card's column under a line across the card, its verdict's chip first, the line 16 px from the
words over it and from the answer's own (`ANSWER_AIR`: the words over it go up a few pixels
for the room, and over a rewrite the answer's words keep 8 px from its box; it stood 12 px
under a comment and 10 over "Claude suggests a rewrite"): it stood in a box of the
buttons' colour whose words were 10 px from its edge. Edit and Delete stand at the end of a
card's buttons where all five fit, and where they don't, a "⋯" there opens a menu of the two:
on a row of their own, a narrow card had three rows of buttons.
A reply has them too, as plain words at the end of the line that says who wrote it, while the
pointer is on that reply (`threads::reply_edits`): Edit and Delete on the reviewer's own, only
Delete on an agent's, whose words aren't the reviewer's to change, and the "⋯" in their place
where they'd reach the author's name. Edit opens the reply's words in the box replies are
written in, the caret after them, Enter saving and Esc leaving them as they were; Delete asks
first. An edited reply an agent hasn't been told of goes with its new words; one it was told of
isn't sent again, as a changed comment isn't. Deleting a reply an agent was told of counts it out
of `told`, so nothing it had is handed over again; an agent's "done" goes with its reply. Until
23 November 2026 only the comment itself had Edit and Delete (the user: "make it possible to
edit delete the reply").

Whatever can be seen can be talked about. A selection, in code or on a page, gets Comment and Ask
Agent beside it, each naming in brackets the keys that do the same as they're set now (Alt+C and
Alt+A unless changed), and a picture, diagram or formula gets a Comment button under the pointer.
Where they stand is one rule (`App::selection_bar_at`, after 0.11.0's review): in a row of their own
over the first selected line, `LINE_AIR` or more clear of words, lines and the selection; where
the line is in a block, a table or front matter or a paragraph's later lines, that's above the
block. With no such row over it in view, its start scrolled away or only blocks over it, the same
under the last selected line, and failing both, beside the end of that line, then over it. Until
then they stood on the row over the first line whatever was there, straddling a box's edge, over a
label or 0 to 5 px over their own selection, or under the end on a table's lines when the start was
out of view. Over or under, it's the gap next to the block the line is in, never one past
whatever comes after that: under a paragraph with a table after it they stood under the table,
11 px over the next paragraph. In the gap they stand in, their foot is 12 px over what they're for, or their top 12 px
under it where they stand under the last line (`measures::SELECTION_BAR_AIR`); where that would
leave less than `LINE_AIR` on the gap's other side, they stand in its middle (in the middle of any gap
too small for 12 px both sides, they stood 9 px over a selection under a title, 0.12.0's release
review), and so they do past the rest of the line's block, a paragraph's other rows, which they
aren't for. A label of something else right
over the first line, a paper's "Abstract", sends them under the last line first, 12 px under it
in the gap right under it, and with words there too, a table's caption, beside the line's end;
only then past what's in the way: in the gap over the label they stood 54 px from the selection
and read as the label's (0.12.0's release review). Their left edge is at the selection's
start, or at the text column's with the start out of view. In 0.11.0 they leaned no more than a
fifth of a row from their row's middle, which put them 5 to 45 px from the selection scene by
scene, and at the caret's column with the start out of view (its release check, P1). Lifted so,
they can reach into the row by theirs, and a click takes them where they're painted; a terminal
that reports no pixels says a click is in its row's middle, and there their own row takes it. Comments are written in the side
pane, with what they're about in view. Ask Agent sends the comment to the agent as soon as it's
written, with its file, line and all of what was selected, and how to answer
with `cellix reply`, so the answer lands under it. It goes alone, as does Ask Agent on one
comment's card: Cellix keeps a key of the prompt it typed in (its words, however they're spaced,
`cellix_agent::prompt_key`), and the prompt hook asks first whether its prompt is one of those
(`alone <key>` over the socket, answered yes once). Told so, it hands the agent nothing else, and
the other comments wait for the next ordinary prompt or Send. Until 20 November 2026 the hook
handed every waiting comment to every prompt, so an agent asked about one answered all of them.
Inside Claude Code the mod starts the turn with that comment as its prompt, where it started one
with Send's prompt for the hook to fill. A Cellix from before answers that it doesn't know the
request, and the hook hands over everything as it did. Inside Claude Code it puts the file and the whole
selection in Claude Code's prompt instead, a passage set out as a quote. A comment is typed as any
text box is, and every box edits as the file does: Shift selects, the Mac's word deletes, undo. Resolving one
stops marking its words at once; the pane can show resolved comments again, to reopen them. A picture's comment names its file, so the agent can open it. All of
this works the same in code, Markdown, LaTeX, data and notebooks. A PDF or image in the side pane
takes a comment too, from a button or a right click, and a paper's PDF lands it on the source line
behind that place, so it stays put through every rebuild.

Output is seen too, so a failing run needs no copy and paste. A selection in a terminal tab gets the
same two buttons, where they cover none of the output: on the row above the selection when its text
ends before them, else on the row below, their right edge on the selection's as both are painted
(it stood 4 px short, at the chip's side inside its cells), and failing both after
the end of the selection's last row, each as a page's are, `LINE_AIR` clear of the output and the
selection over and under them, and as far from the selection as a page's: their foot 12 px over it,
or their top 12 px under it, or the middle of a smaller gap. Only output with no such room has them
over the end of the row above, still 12 px over the selection, where they stood 6 px over it. A terminal tab selects as a page does, and as terminals do: a drag, copied as it's let go,
and a Shift+click that ends the selection from where it started or from the click before, however
far the scrollback has been scrolled in between, since a selection keeps to its words as they
scroll (it kept to the screen's rows until 18 November 2026, so a scroll moved it onto other words).
With Shift held a press selects even where the program takes the mouse, as every terminal lets it.
A drag held past the top or bottom of its screen scrolls the scrollback as a page does, and the
wheel turned while dragging takes the selection on with it.
Scrolled out of view, the selection's buttons go until it's back. The agent hears all of the output, the command that printed it and every
`file:line` in it that is a file of the project. Cellix knows the command from where the first key
was typed while the shell was at its prompt, read off the screen when it's asked. A comment on output
has no file, so it shows in the Comments tab whatever file is in view, and opening it goes to the
first place it names. A row of the Problems tab gets Ask Agent under the pointer, which sends what the
checker said, the file and line, and three lines of code either side. In the code, every problem
the tab lists, a language server's or a task's, marks its line the same two ways: the line's number
in its worst problem's colour, a warning's as well as an error's, and the word the problem starts
at underlined in dots of its colour (its cells underlined on the cell tier), or the line's letters
where it starts on none. A problem says only where it starts, so its word is what's marked. Until
19 November 2026 the number was red for a language server's problems alone and nothing was
underlined (132-task-problems: line 2 red with nothing under `count`, line 1's warning unmarked). With no file in view the tab lists every file's
comments, so each file's stand under a heading with its icon, its name and how many, as in VS Code's
Comments view: a file at a time by name, a terminal's output last. A click on a heading opens the
file, and the list is then that file's alone.

### Ask, a side question

A question isn't a comment. The user selected "ICL" on a paper, pressed Ask Agent and asked what
it meant, and the agent answered every comment in the review: Ask Agent made the question a
comment and typed it into the agent, and the prompt hook handed every waiting comment over with
it. A comment is feedback the agent is to act on, counted, sent and answered under its words; a
question wants an answer, now, for the reader, and nothing done (approved by the user, 20
November 2026).

So Ask runs the agent's own command line once, in the background, non-interactive and
read-only, in the project's folder (`ask.rs`). It never types into the agent working in the
side pane or disturbs its session, and it brings in no comments. Which command line: the
Default agent, a row of Settings › Integrations (Auto, Claude, Codex, Gemini) over the cards,
kept machine-wide as `ask_agent` (`default_agent` names it too); Auto, or nothing set, is the
kind of agent in the side pane where that's installed, else the first installed of Claude
Code, Codex and Gemini CLI. Polish and Validate run the same one (25 November 2026). With none,
the card says what it needs and has Open Integrations, which opens Settings there; with the
Default agent one that isn't installed, it says that. Integrations scrolls whole, as
Appearance does, so every card under the row stays in reach on a short window. Kiro CLI doesn't answer: its
`chat --no-interactive` needs a `KIRO_API_KEY` (paid plans) rather than the person's sign-in, runs
on its V2 harness unless told `--v3`, which a Kiro CLI before 3.0 doesn't know, names its
read-only tools only by example (`read_file`, `grep_search`; V2's `read` group from 3.3), and
documents of its `stream-json` output only how a run fails (22 November 2026, from its docs).
With a Kiro CLI tab in front, Ask goes to the first of the three that's installed.

What it knows, by layer:

1. The document whole, with the selection marked ⟦like this⟧, the headings it's under ("The
   selection is under: Notes › Method", a paper's numbered as LaTeX numbers them, "2 Method ›
   2.1 Training"), its path in the project, so it can read the bibliography or anything else,
   and a paper's `\input` and `\include` files read in where they're named, each between lines
   naming its file. Past 150,000 letters (`WHOLE_LIMIT`), it gets the widest section round the
   selection that fits, else as much round the selection as fits, after the document's outline
   with each heading's line, and is told the rest is in the file. Output selected in a terminal
   tab goes with the tab's name, the command that printed it and the project's places it names.
   The rules come first, every time: answer only this question, briefly and for the reader;
   change, create or delete nothing; ignore any review comments.
2. Read-only tools. Claude Code: `-p --output-format stream-json --verbose
   --include-partial-messages --allowedTools Read,Grep,Glob --disallowedTools
   Bash,Edit,Write,MultiEdit,NotebookEdit`. Codex: `exec --sandbox read-only --json
   --skip-git-repo-check -C <root> -`. Gemini CLI: `--output-format stream-json --approval-mode
   default -p …`; without a person to ask it leaves out every tool that would need one, the
   shell, edits and writes. Every prompt goes on the program's input, never as an argument,
   which a whole document would take past Linux's 128 KB for one. Those options were read from
   `--help` on this machine (Claude Code 2.1.311, Codex 0.166.0); Gemini CLI isn't installed
   here, so its are from its documentation and unverified.
3. One side conversation per document. The first question records the session the command line
   gives (Claude Code's `init` message, Codex's `thread.started`, Gemini's `init`); a later one on
   the same document, on any of its words, resumes it (`claude -p --resume <id>`, `codex exec
   resume <id> -`, `gemini --resume <id>`), so it remembers earlier questions and the document is
   mostly cached. The ids are kept in `asks.json` with Cellix's own state for the project (beside
   its sessions, `hold::folder_of`), never in the project's files. Each keeps the hash of the
   document's text as it last went: unchanged, a later question sends only the words, where
   they are and the question; changed, it resumes and sends the document again as it reads now,
   saying it changed, since the earlier questions still mean something. A resumed conversation
   that fails before saying anything (its command line forgot it) is forgotten and the question
   asked once afresh. Another command line than the kept one starts its own.

4. With the agent's conversation, a choice under the field where a Claude Code agent working here
   has a known session (the side pane's, by its hooks, or inside Claude Code the one the pane is
   in): it asks in a copy of that conversation, `claude -p --resume <id> --fork-session`, with the
   read-only tools, so it knows the work and the agent never sees the question. Its follow-ups go
   on in the copy. Codex 0.166 has `codex exec fork`, but Codex's sessions in the pane aren't
   offered yet, and Gemini can't be copied.

Every hook ignores a side question. The run carries `CELLIX_SIDE_QUESTION=1`, and under it
`cellix hook` does nothing for any agent or kind (`hook::handle_for`, and before it the binary's
own session-start work), and `cellix pane`, which Claude Code's mod starts as a session starts,
doesn't start. Otherwise the side question would collect every waiting comment, the very bug,
take a checkpoint, log a turn and show up in the Agents list. It also leaves out what says it's
held by Cellix or named by it (`CELLIX_HELD`, `CELLIX_NAME`) or inside another Claude Code.

On screen: a selection's buttons are Comment and Ask, on pages, in code and on a terminal's
output, each naming its keys (Alt+C, Alt+A). The Problems row's Ask Agent still sends the problem
to the agent to act on, and a comment's card keeps Ask Agent, which sends that comment alone
(see Companion). Ask opens a card under the words, standing as a comment's glance card does,
under them where there's room and over them where there isn't, never on them, its words where
theirs start: "Ask Claude Code" at its head, naming who answers, a field whose hint is "Ask
about ‘ICL’…", and Explain, Define and Simplify as quiet buttons before a filled Ask. The
selection's own Comment and Ask give way to the card while it's about those words.
Enter with nothing typed explains; Esc or a press elsewhere closes it. Asked, its head is the
question with who answers at its end, quietly, a spinner turning before the name, and the answer streams in as it comes, Markdown
shown as a comment's words are: plain, a list's items as bullets. While nothing has come, it
says what's going on ("Thinking…", "Reading refs.bib…"). Past 14 rows the wheel scrolls it. Its
buttons: Copy, Follow Up (a field in the card that goes on in the same side conversation), Make
a Comment, and Dismiss (Esc). It stays while the file is worked on, so it can be read beside the
words, and goes when dismissed. An answer is never a review comment: never counted, never in
Send N, Check My Review or the prompt hook's hand-over. Make a Comment makes one of it, for when
it's feedback after all: the question as the comment, of kind Clarity (a word that needed
asking about is a clarity finding; the five kinds have no Question), on the same words, with the
answer as the first reply by the agent that gave it. Its last word is the agent's, so it waits
for nothing until the reviewer replies. What goes wrong is said on the card in words, with Try
Again: not signed in, the command line's own failure, or no answer in four minutes
(`TIME_LIMIT`), after which it's stopped. Inside Claude Code Ask is the same side question,
where Ask Agent had filled Claude Code's prompt box.

### Reviewing a remote doc

Docs that live behind a doc tool Cellix can't reach (a web app behind a security key, which
the person's agent reads through an MCP server they already have) are reviewed as a silent
review is: read, ask, write a comment, polish it, and post it, one by one, only when chosen
(approved by the user, 24 November 2026; board "Reviewing a remote doc").

The agent is the bridge. Cellix never speaks the doc tool's protocol and doesn't know its
tool names: it runs Claude Code in the background, as Ask runs it (`ask::start_run`, the run
carrying `CELLIX_SIDE_QUESTION` so no hook stirs), with a fixed instruction, and the agent
answers with data that Cellix checks and writes itself (`remote.rs`, `app/remote.rs`).

- Which server: those Claude Code knows, read and never written: `mcpServers` in its
  `.claude.json` (in `CLAUDE_CONFIG_DIR` when set, else the home folder), the ones it keeps
  for this project there, and the project's `.mcp.json`. The machine-wide setting
  `remote_docs_mcp` names the doc tool's; the first review asks, with a menu of them, and the
  strip's menu has Read Through Another Server…. A test names its own config (`Remotes::env`)
  and without one reads none.
- What a run may do: `claude -p --output-format stream-json --verbose --tools ""
  --allowedTools mcp__<server>__* --disallowedTools Bash,Edit,Write,MultiEdit,NotebookEdit,
  WebFetch,WebSearch,Task,mcp__<every other server>__*`: no built-in tool, only that server's,
  never `--dangerously-skip-permissions`. The server's name is made as Claude Code names its
  tools, anything but letters, digits, `_` and `-` as `_`. `--tools` was read from `claude
  --help` on this machine. Polish runs the Default agent, not the reader: Claude Code with no
  tools at all, `--tools "" --strict-mcp-config`; Codex and Gemini CLI, which have no flag
  that takes every tool away, read-only as Ask runs them (`ask::toolless_args`). Reading and
  posting stay Claude Code's whatever the Default agent is.
- A read is told to use the tools that read only and to answer one JSON object, `{title, link,
  version, markdown, comments: [{id, author, quote, text, at, resolved, replies: [{id, author,
  text, at}]}]}`, or `{"error": …}`; times may be ISO 8601 or seconds. Cellix writes
  `.cellix/remote/<slug>/doc.md` and `remote.json` (title, link, version, server, when synced,
  how many comments are others'); `.cellix/remote` has a `.gitignore` of `*`, since it's
  someone's company's content. The slug is the link's words and six letters of its hash.
- Others' comments are threads with an `author` (empty for the reviewer's own) and a `remote`
  of the doc's id and a state, `draft`, `posting`, `posted` (when) or `failed` (why); replies
  carry the same. They're anchored by quote; a remote thread is never handed to the agent at
  work here with a prompt (`Thread::waiting`), and Ask Agent sends one when pressed.
- The doc opens as a page, read-only, its tab named by its title with a cloud in the accent.
  The row under the tabs is its strip: a cloud, "From <server>, read by Claude Code", then
  quietly the version, "synced N ago" and how many comments are others', and at its end Sync,
  Open in Browser (the system's browser, where the doc tool's sign-in is, never the pane's)
  and a menu with End Review. A workspace's remote docs reopen next run from `.cellix/remote`.
- Cards: someone's says "Priya · in the doc", the name bold in the words' colour as a reply's
  author's is (the reviewer's "You" stays plain), with Reply, Ask Agent and Open in Doc. Post
  to Doc, Post Reply and Retry carry a paper plane before their words and Copy two sheets, as
  the board draws them. The server menu opens with a row lit for Enter. The
  reviewer's is a draft until its own Post to Doc (filled), then Validate, Polish, Copy, Ask
  Agent and Reply, and Edit and Delete as plain words at the end: the same buttons in the same
  order as the reviewer's card on a page of the project, which has Resolve after Reply where a
  remote one, resolved in the doc by the doc tool, has none. Posted, it has Validate, Copy,
  Ask Agent and Reply, then Open in Doc: its words are the doc's, so no Polish, Edit or Delete.
  A card's buttons flow over rows where one doesn't hold them, and a short card keeps one,
  the rest behind its "⋯". Its pill says "Draft, not posted yet", "Posting…", "Posted N ago" in green, or "Not
  posted" in red with why under the words and Retry and Copy. A sign-in the doc tool wants
  again reads "The doc tool asked to sign in again. Sign in (your security key), then Retry."
  The list's head says "Yours: N posted · N drafts" and "Others: N" in place of Check My
  Review and the kinds. The composer's button says Save Draft.
- Nothing posts but that press, or Post All Drafts once its question is answered Post. Not
  Sync, not Enter: a post carries one comment, its quote and words, and asks for `{"id": …}`;
  a reply goes under its comment by the doc's id. One posting or posted isn't posted again;
  one under way when Cellix closed comes back failed, saying to look in the doc before Retry.
- At the top of the Comments pane, on a page and a remote doc alike, under its head: Validate
  All and Polish All, each where a comment of the reviewer's has nothing from it yet (Polish
  leaving posted ones), and on a remote doc Post All Drafts (25 November 2026). Validate and
  Polish run two at a time; Post All Drafts first asks "Post 4 comments to the doc?" with Post
  and Cancel, then posts one at a time, each as its own Post to Doc would, a failed one saying
  why on its card while the rest go. While one runs the row says "Validating 3 of 7" with a
  spinner, and Stop, which lets what's under way finish and starts nothing more. A press on a
  kind's pill lists only that kind, and the row acts on what the pane shows; the buttons flow
  over rows where the pane's head has no room, each whole, never cut short, as a card's do.
  The question's detail wraps in the dialog's width, and the dialog grows with it.
- Polish runs on the draft's words, its quote and the paragraph round it ("make this review
  comment short, clear and kind; keep its meaning; answer with the text only"); the
  suggestion stands under the draft in a box of the page's ground, "Polished · Claude Code ·
  17 words, from 41", with Use This and Keep Mine. It's still a draft either way.
- Validate, on the reviewer's own comment here or on a page of the project alike
  (`app/checks.rs`): the Default agent with no tools, given the whole document, the words the
  comment is on and the comment, answers one line, `Verdict (Kind): why`, the verdict Holds up,
  Doesn't hold or Already answered (naming where), the kind one of the five. It stands under
  the comment in a box as Polish's words do, its verdict on a chip, green where it holds up and
  the warning colour otherwise, then the kind and who said it, why, and Dismiss. It never
  changes the comment, its words or its kind. Both may stand at once, Validate's first.
  What each said is kept with the comment in the comments file (`Thread::validation`,
  `Thread::polished`), with the words it was said of: so it's there after a Sync, a restart or
  the doc opened again, and goes once the words change (Edit, Use This). A run under way or
  failed is only on screen.
- Sync reads again: comments and replies new by id come in, someone's edited or resolved ones
  are brought up to date, `doc.md` is replaced and every thread found again by its quote (one
  whose words went says so). Drafts are never touched or posted. End Review asks, then deletes
  the folder, its tab and its comments.

Claude Code only, for now: Codex has no way to allow one MCP server's tools and refuse the
rest in `exec` (its MCP servers are all-or-nothing per config, so a run would need a config of
its own naming just that server), and Gemini CLI's `--allowed-mcp-server-names` would do it
but its MCP tools still ask for approval it can't get in `-p`, unless trusted. Both are left.

## The pointer

Cellix tells the terminal which pointer to show with OSC 22: a hand on what can be pressed, a bar on
text, an arrow elsewhere. Left to itself a terminal picks one from its own state, which can stick.

A link takes the hand: on a page or in a terminal while Ctrl is held, since that's what a press
there needs, and on a comment's card, where a plain press opens it. Ctrl held is learnt from the
key and mouse events' own modifiers; a terminal sends nothing when Ctrl alone goes down or up, so
the shape follows at the next event — the move that takes the pointer to the link, in practice —
and it's told again each time a frame is drawn, so one message lost costs one change, not the state.

Where the terminal reports the mouse in pixels, a press is turned into a cell by the cell's size,
so that size has to be the terminal's own. It's asked for at the start (`CSI 16 t`) and asked
again whenever the window's size in cells or in pixels has changed and held still for a fifth of
a second, by a terminal that answered the first time. The window's pixels over its cells is only
the fallback: a terminal that counts its padding or a tab bar in the height it reports makes
every row seem a few pixels taller than it is, and a press low in the window then lands lines
above the pointer. That guess used to replace the terminal's answer at every resize.

## Guidance

Nothing should leave someone wondering what to do next. An empty window lists its actions as rows
that can be clicked, and its first line changes with what's going: with an agent running and no
file open, it says to click a file on the left, or in the compact layout, to pick one from ☰. The
agents panel says how working beside an agent
goes before offering to start one. The first start says how to move between the two sides. Every
panel says what would fill it. Getting Started is a real page, so reading it shows what a page does.

## Documents

A Jupyter notebook opens as a page too, with what each cell printed and drew under it. Its cells
become Markdown with a hidden line above each, and saving puts the text back into the JSON beside
the outputs it already had. Cells run in a real Jupyter kernel, through a small Python program that
passes lines of JSON between Jupyter's client library and Cellix, using the project's own Python if
it has one. Outputs land under each cell as they come, and are saved with the notebook. A notebook
gets its own bar with Run, Run All, Interrupt, Restart, adding cells and what the kernel is doing,
and Jupyter's keys: Shift+Enter runs and moves on, Ctrl+Enter runs and stays.

CSV and TSV files open as a table on the same page, with cells edited in place and rows and columns
from the menu. A long file shows its first 2,000 rows and says so. Every table moves like a sheet:
Tab and Shift+Tab go along the row and select what they land on, so typing replaces it, Enter goes
down the column, and going past the end adds a row.

Spelling and grammar come from Harper, which runs here with no network. What isn't prose is blanked
out first, keeping every char in place, so a finding lands on the right word in the source. It runs
on its own thread after a pause in typing. Words added to the dictionary go in
`~/.config/cellix/words.txt`. A document that doesn't read as English isn't checked.

Pages read more than CommonMark: strikethrough, `==highlight==`, `H~2~O`, `x^2^`, footnotes, GitHub
callouts such as `> [!NOTE]`, and CriticMarkup for tracked changes. All of it is still plain text in
the file, and it reaches the web page and the PDF. A reviewer's note in the text (`{>>note<<}`) is
shown as a comment is, three spaces after the words before it (two on cells), after the comment's
◆ and on a comment's tint, in the small words; until 0.11.0's review it stood against the period
before it in small grey words and read as a typo. The tint keeps `LINE_AIR` before the ◆ and after
the note's last letter, its spaces at either end left out; it had 6 px before the ◆ and 11 after a
row's last space. Where the note wraps, the tint is cut flush there, as a browser slices an inline
box: a row it runs on to starts its tint on the text's column, and a row it runs on from ends it
at its last letter. With 8 px on every row, the row it ran on to put its tint 8 px into the page's
margin, where nothing else goes (0.12.0's release review). A
line under words, as an insertion's, or through them, as a deletion's, runs from the first letter
to the last: under the space at an insertion's end it ran on under the next word. A horizontal rule is one hairline across the text's column, as
wide as its words are set; its dashes are the cell tier's (in 0.11.0 they were drawn over a shorter
hairline too, a line that changed colour part way and stopped short in a wide pane). Track Changes
keeps edits as suggestions in the text itself, as CriticMarkup or the `changes` package, so they
travel with the file and any tool, agents included, can read them.

A suggestion is settled where it stands. While the caret is in one, or the pointer is on one,
Reject and Accept stand over its first row from where it starts, Review's two words for the same
thing, with Accept filled as it is there; under its last row when the first is the top of the page,
which Next keeps it off. They keep to the page's reading column, and lean a little toward the
suggestion in the row's air. They're the chips a selection's Comment and Ask Agent are, and give
way to them while anything is selected; edged and filled, with no shadow, since the shadows of two
side by side met in the gap between them as a band of grey and stood out past them with a hard
top edge. A selection's Comment and Ask Agent, and the pair over a terminal's output, are the same
since 0.11.0's release review: theirs made the same band, #dadada on white, 10 px past Comment. The one under the pointer comes first, and stays while the
pointer goes up to its chips across the row's other words. A press settles it as one edit to undo
and says how many are left. Until 18 November 2026 Accept and Reject were only in the right-click
menu and the palette, where they still are, with no key: the user asked for them at the change.

A document with suggestions opens the Comments pane's list with them, as Google Docs and Overleaf
keep suggestions with comments: "3 suggestions", or "Suggestion 2 of 3" while the caret is in the
second, then Accept All, Reject All and Next, quiet as every button there is, on a
row of their own under the count where the pane is too narrow for one. Next goes to the next one
round the document and leaves the keyboard in the file, where the chips then stand by it. The row
is the document's in view, so the list of every file's comments has none. The suggestions are found
once for each revision of the text, since every frame asks for them.

An agent's rewrite of commented words (a reply starting `Rewrite:`) offers Accept Rewrite, Apply as
Suggestion and Not This. Accept Rewrite puts it in place of the words as one edit to undo and
resolves the comment, as committing a suggestion on GitHub resolves its conversation; Apply as
Suggestion puts it in as a suggestion to settle on the page, which was the only way until 18
November 2026. Both replace all of the commented words as they read now, found as a comment's
words are (below). Where the three don't fit a card's row they flow left to right, as many on
a row as fit: Accept Rewrite kept alone on a narrow card's first row had 400 px free beside it.
The rewrite stands in a box on the card's columns, 16 px round its words and 16 px over its three
buttons. Accept Rewrite is the card's one primary, filled, the other two quiet: quiet beside a
blue Check My Review, it read as one choice among equals (0.11.0's release review). Check My
Review is quiet too, with its agent's icon in its words' colour, so one button in view is filled
(0.11.0's release check, P4: both were blue). Its box
had its words 10 px from its edge and stood 10 px over its buttons, and the two rows of buttons
under it were 23 px apart where a card's are 11. A paper's suggestion whose words run onto another
line shows on its page as one, as a Markdown one does (`Spanning`), where a line's own command left
a brace and lost the new words; across lines its new words stand before the old, as they do in its
source and its PDF.

The Outline in the sidebar lists a document's headings, or a paper's sections, and a click goes
there. The side pane keeps what the last build said in a Log tab, and a line that names a place
goes to it. Wide glyphs are looked up as sans-serif rather than monospace in the cells: asked for a
monospaced Han glyph, the font search stops at a font that holds only blanks.

Diagrams are text in the document. A fenced `mermaid`, `dot` or `svg` block stands on the page as
a picture, and opens to its source with the picture under it while it's written, 16 px under the
source's box as a drawing's note is: a formula's numerator touched the box (46). A pie's slices
and swatches take colours 3:1 or more from the page in both themes, where the renderer's first was
white on white (67), and the slices are drawn whole, as the swatches are: at the renderer's 0.7 to
0.85 opacity no slice was its swatch's colour. A state diagram's labelled edge to the state right under it goes straight
down with its label beside it, 8 units clear of the line, in the middle of the gap: the renderer
leaves that gap the label's height and routed the edge round its own label, so the arrowhead
stood on the label's foot (66). Cellix lays a diagram out with the renderer and changes that
much of its layout before it's drawn. They are drawn by
pure Rust (`mermaid-rs-renderer`, `layout-rs`, `resvg`), so nothing needs a browser or Node, and
the web page and PDF export carry them too. What only LaTeX can draw (TikZ, `tikzcd`, `forest`,
`circuitikz`, `algorithm`) is built alone in the background with the paper's own engine and setup,
and kept by its content in the cache folder, so typing never waits. Until it's drawn its source
stands in its box, with "Drawing with LaTeX…" 16 px under the box and a thumb for its width inside
the box, 8 px clear of the box's border at its sides and its foot and of the code over it, where
the thumb ran through the note (0.11.0's release check, P4), then stood 2 px from the border 4 px
inside the box's edge (0.12.0's release review). A box wider than the page reaches 5 px further
under its last line to hold it (`THUMB_ROOM`, laid out with the page so what's under the box keeps
its air), where it stood 6 px under a "$$". A code box's or a formula's source's is the same; a
table, which has no box, keeps its thumb under it.
LaTeX that gives no answer in a minute (`snippets::LIMIT`) is stopped, with whatever it started,
and the note says so where it waited for ever; what held it up may pass, as a first fetch of
packages does, so each session asks once more. With no LaTeX installed the note says that. Line art becomes ink in the
sheet's colour, so it reads in both themes. An image's size is written as `{width=50%}`, which
Pandoc reads as well, and as `width=0.5\linewidth` in a paper. Dragging an image's edge sets the
same thing, in steps of five percent.

Markdown and LaTeX share one page: `rendered.rs` lays out Markdown and `rendered/tex.rs` lays out
LaTeX, into the same rows of glyphs that each remember their source position. So clicks, selection
and editing work the same in both, and the file is always the document.

LaTeX on the page is a reading of the source, not a compiler. Commands it knows hide, and anything
else shows as source, so a paper can never lose text to the page. Numbers come from a first pass
over the labels, as LaTeX's own does. An equation's number stands at the right margin level with
its row's math axis, a quarter of an em over the row's baseline, as each row measures alone: on
the middle of the row's height, limits under it pulled the number about 30 px under its line. The
PDF is the truth for how a paper prints. A page or picture zoomed in the side pane keeps 16 px
from the pane's sides however far it's zoomed (`pdfview::SIDE_AIR`).

Formulas are pictures made by `latex-rust`, in pure Rust with STIX Two Math in the binary, because
pixels are drawn on the host and a server has no TeX. A picture holds one place in the text, inside
its source, so reaching it from either side opens it. PDFs draw with `hayro`, also pure Rust.
Building a PDF uses a real engine, since only TeX typesets TeX, and Cellix offers to install one.

A comment on a document is a thread kept in `.cellix/threads.json`. Comments on plans and `--wait`
files also go back as the answer the agent is waiting for. A plan's threads move onto its next
version: words still there, or only rewrapped, take them along. A thread on words that went stays
open, marked with the version that removed them, and leads to what changed. Words reworded are
not taken as the same, since one word can turn a sentence around: the thread keeps the words it
was about, is marked as changed in that version, and leads to what changed too. A negation or a
number in or out never counts as a reword. Approving a plan settles its threads: they're resolved
and kept with the approved copy in `.cellix/plans`, and the hook puts that plan's versions away in
`Approved N`, so the next plan the session sends starts at version 1 with no threads.

A comment reaches an agent one of three ways, and its card says which until it has. One on code
goes with the review's comments; one on a document is a thread no agent has had until it's told.
The prompt hook of Claude Code, Codex or Gemini CLI hands over everything waiting with the agent's
next prompt; Ask Agent sends one now; and Send Comments to Agent sends all of it, the review's
comments and the threads, in one message to the agent the side pane sends to (inside Claude Code it
starts a turn, which the hook fills). Until 18 November 2026 it sent only the review's, so on a
page it said there was nothing to send. A waiting card says "Goes to Claude Code with your next
prompt, or now with Ask Agent"; for an agent whose hooks can't hand comments over (OpenCode,
Copilot CLI, Cursor's agent, Kiro CLI) that only Ask Agent sends it; with no agent here, that it goes with
the next one's prompt. Once it has gone, "Waiting for Claude Code…" names the agent it went to:
the one in front of the pane, which is the one sent to, where it named the first agent there.
Check My Review asks for a verdict on each comment, so each counts as told and the hook doesn't
add them to that prompt again. A suggestion kept is the person's comment, and goes as theirs do.
Under Check My Review, ending where it does, a quiet "Send 3 to Claude Code" sends what Send
Comments to Agent sends, as GitHub's Finish review does, where there's an agent to send it to now:
in the side pane, or the Claude Code Cellix is a pane of. It counts the review's comments and the
threads no agent has had, and goes once they've gone.

A comment is about all of what was selected, over as many lines as it took: its quote is the
whole selection, found again across line breaks and runs of spaces (`threads::span_from`), so
words selected over a hard-wrapped paragraph are marked on every row they stand on, quoted whole
to an agent asked to check the review, and replaced whole by a rewrite applied. Until 18 November
2026 the quote was the selection's first line, and Apply replaced that line with the whole
rewrite, leaving the rest behind. A comment keeps the words it was made on, which its card
quotes. When an agent rewords them, the words as they read now are kept beside them (`now`) and
are what the page marks and a click selects, and the card says "Changed since your comment";
when they're gone, it says "Words gone", strikes the quote through and marks nothing. It used to
take the new words as its quote, so a card showed words nobody had commented on. An agent told
of it hears both, the words commented on and what they read now. A word or two changed is
followed over as many lines as the words take, and never onto a "not" or a number in or out.

## Surfaces

There are two surfaces: the frame, and the sheets on it (the user's choice of 16 November
2026, after a picture of Lapce; until then there were three, the sidebar and the side pane a
step between the frame and the file). The frame is the views, the sidebar and the bar at the
foot, in the terminal's own colour, so window padding can't show as a seam round them, and
nothing is ruled off on it. The file and the side pane are sheets: one colour, their tabs on
it with the accent's line under the one in front, a column between them that is the frame's.
With pictures a sheet has a line round it and round corners, and its edge stands half way
across that column, so what's in the pane keeps half a column from it, and its tabs start
past the column, a whole one from the sheet's edge; past the grid's right
edge a row runs on in the frame's colour (`Grid::edge`), so a sheet ends where the grid does.
Under the file, the pane is the lower part of the file's sheet, with a line between them.
Without pictures the sheets are square and there's no line round them, and a tab in front
has its cells underlined. Every surface moves together when the terminal's background does,
so the step between them holds on any colour. The sidebar is drawn in `Theme::on_frame`: the
panels' colours on the frame's ground, with what washes a row moved as far.

Paint can't go under text, so a rounded surface is coloured cells with its corners painted
back in the colour behind. Icon pills work the same way. Floating panels reach past their
rows in paint, which gives them padding that a row of cells couldn't. A card whose panel starts
and ends inside its first and last rows hides those rows' words whole, so none shows half
covered, but not a picture or a drawing: that shows whole up to the card's line, and the card
hides only what it covers (`Grid::pictures_again_in`). An equation under the Ask card's foot
lost 29 px of its first line (0.12.0's release review, q4-ask-follow-up).

Ghostty doesn't clip images, so on Ghostty tiles reach past the text area: each row runs on
into the right padding in its edge colour, and the last row runs on to the bottom of the
window, with the status text moved down to the middle of that band. Left and top padding
can't be reached, since an image can't start before the grid, and that's why the frame there
is the terminal's own background. Other terminals clip or keep this to themselves, so
nothing changes there. Ghostty's config is never touched.

A tile is keyed by what its row shows, and paint can stand in several rows, so a row with
paint on it keys by its own number too.

### Git beyond staging

Every row of the code view is a line of the file, so nothing is drawn between lines. A merge
conflict is a card made of its own marker lines: `<<<<<<<` is the header, naming the current
side and holding Compare, Both, Current and Incoming; `=======` names the incoming side;
`>>>>>>>` is the foot. Current is tinted green, incoming blue, both mixed from the theme.
Incoming is the filled button; the header's other three are the quiet kind every other action is,
raised off the green band in the dark theme (toward graphite's sheet they were darker than the
band and looked sunk), toward the sheet in the light one, where that's lighter.

The commit graph sits at the end of Source Control, as in VS Code. Its lanes are straight
lines with square joins, box-drawing characters on cells and hairlines on pixels, and a merge
is a hollow dot. A commit opens to its files under it; a file opens the diff view against the
commit's first parent. The open file's commits are the Explorer's Timeline.

The branches and tags at a commit stand before its message as chips, as VS Code's graph has
them: each name in its commit's lane colour on a wash of it, in a chip's words and at a
chip's height, its words in its middle by their font's span. They share the row with the
message: they take what leaves the message 14 columns, and at least half of what the two
share, so in the sidebar, 28 columns at most, both are cut rather than the names left out
(as plain words, and only where the message kept 14 columns, no name ever showed there). A
name too long for its room is cut in its middle, as the merge panel cuts a branch's, so
"agent/queue-bound" keeps both ends, and one that can't keep three letters isn't shown, nor
any after it. On cells a chip is its name on the wash. In paint the message runs to a column
before the age's measured words; the age's count of cells left 72 px empty after it.

Commits, stashes and Compare With use the diff view read-only: no Accept or Reject, a list of
what's in them, and a title naming both sides. A stash is named by its commit, never its
place in the list, since every worktree shares one list and agents push and pop it.

## Theme

`theme` is `auto`, `dark` or `light`. `auto` is the default: Cellix asks the terminal for
its background with OSC 11 and takes the matching theme, staying dark if nothing answers.
Dark is herdr's graphite and light is Catppuccin Latte's surfaces (under Look).

## Quit, at the foot

The button that quits Cellix is the power mark at the far end of the status bar, after the
bell, with a line and two columns between them so a press meant for one can't land on the
other. In cells it's the word Quit. Its tip, on the row over the bar, says "Quit Cellix
(Ctrl+Q)". The first screen has it in the same place.

It was an × at the top right until 16 November 2026, in the pane's tab row beside the
control that hides the pane. The user said it read as closing the workspace, not Cellix:
every other × on screen closes the thing it's on, a tab or a workspace's tile. So it left
the sheets for the frame, which belongs to no tab, pane or workspace, and took a mark no
other control has. A window under 40 columns goes without it, and the key still quits.

## One bar at the foot

The workspaces lead the status bar's own row (the user's choice of 16 November 2026: two bars
with chips of two kinds were the first thing they called clutter). From the left: a tile for
each workspace, the one in front in the accent with its words in the colour that reads on
it, the button that opens another as its mark alone, a rule, then what the status bar says:
the branch, the count of problems, what was last done. At the far end the file's details,
the machine's name, the bell, and the button that quits. `Workspaces` tells each `App` the columns it has taken
(`status_room`: the lead, and the room before the bell), and the status bar starts and stops
short by them. A press on a workspace or Resume is the bar's; anywhere else in the row it's
the status bar's. What the status bar painted is raised with the bar's own, so the two stand
level and keep their air at the window's edge.

The bar has a row of its own under the status bar, as it had until then, where fewer than 72
columns would be left between the workspaces and the machine's name, Resume and the bell: a
narrow window, or many workspaces. That's what the status bar gives a message, 64 columns,
and a branch before it; with less, a message was cut short (the real-binary tests caught
one). It has a row of its own too where what's left wouldn't hold the status bar's words at
their fullest, measured as they're drawn: a branch, a language server's wait and a file's
details (`App::status_full_room`). The status bar's words come before the workspaces'. Those
are fixed words, not what it says now, so the foot keeps its rows while a server starts or
stops fetching. With the 72 columns alone, and Resume and the bell not counted, the tiles had
the row once they were narrower, when agents outside Cellix stopped giving them words, and
"Spaces: 4" went while rust-analyzer fetched (the review of `next` before 0.11.1). There its
button says New Workspace. Without pictures there's no chip, and the workspace in front
has the brighter name.

A workspace's tile shows what its agents are doing with the marks their tabs have: the spinner
while one works, and a dot in the colour of what needs the person (#f0705f) while one asks, at
the size of a spinner beside the status bar's words, standing in the room a word's mark has.
On the tile in front, which is the accent, the spinner is in its ink, and every dot keeps its
own colour in a ring of the ink 2 px wide: plain, the needs-you dot stood on the accent at
1.23:1 (1.09:1 light), and the green and dimmed dots were the ink's, telling nothing apart
(0.11.0's release review). Until 0.11.0's review the tile said "working" after
a still dot, and "needs you" in the chip's ink with nothing of its colour. It has those marks
and only those: a green dot for a finished turn, where it had a check, a dot dimmed as an ended
tab's mark is once it stopped (it had none beside chips that each had one), and no mark where
a tab would have none, idle or empty, where it had a dark dot and a ring no tab has; with no
mark its name has the chip's padding before it. The status bar's "An agent needs you" has the
same dot at the same size, where a dot half that size stood. What it says is the most urgent
thing any of its tabs says, and the badge on the Agents icon and the status bar count the same
way: a tab asking stood beside a tile that said "done just now" once another agent's news from
the same process had taken the asking one's place among the agents (`App::needing_you`). What
waits in a workspace's Inbox and no tab's mark shows, a failed test run or an agent's answer to
a comment, gives its tile the needs-you dot too, so the bar names where the next step is: a
tile had no mark while "1 of 14 tests failed" waited in its Inbox. Only agents the Agents list
shows count anywhere, the tiles, the Inbox, the badge and the bell alike: one outside Cellix
has nothing here a press could act on. The
first screen, the workspaces to pick from, shows the same marks, a spinner where it had a
still dot, and Open and Quit are buttons, the main one filled and Quit quiet, where they were
words.

Under the sheets the frame fills a band: the bar's own row, what the terminal's lines leave
over under the last of Cellix's rows, and the terminal's padding under its lines, which
Ghostty lets a picture run over (`Bleed`). The bar stands in the middle of that band
(`PixelBackend::bled`: half of what's left over, known to the pixel, and of the padding,
which isn't reported and is taken as what Ghostty is set to and half a line). Until then it
stood at the band's top with all of what was left over under it: the user's first point on
trying 0.10.0, "bottom there is too much space". A press under the last row is a press on it
(`on_the_grid`).

Where the workspaces have a row of their own under the status bar, the foot is two rows, and
the grid says so (`Grid::foot`). They stand evenly in the band (`stood_apart`): a third of
what's spare over the status bar, a third between the two and a third under the workspaces.
With only the last row moved, all of it lay between them: 12 px under the sheet, 40 between
the rows, 35 under. The status bar's tile is grown by the pixels it stands below its place,
and the workspaces' is placed where that tile ends, so nothing is drawn twice and no tile
lies over another. A row at the foot is then drawn below where it begins, and a press is on
the row drawn under it (`on_the_foot`, from `Backend::foot_drops`): half of what's between
the two belongs to each. Rows that are the terminal's own lines can't stand off them, and
there only the last row is moved, into the padding.

Evenly apart as rows, the two weren't as what's drawn: the status bar draws on 30 px of its
50 and a chip is 37, so there were 28 px over the status bar, 35 between the two and 26 under
the chips. On rows with room the status bar's paint is 0.04 of a row lower in its row and
the workspaces' 0.06 higher (`PAIR`), which makes that 30, 29 and 29.

A tip over the bar, the app's or the one over New Workspace, stands high in its row on rows
with room, its words raised with it, and not in its middle. Its shadow reaches 11 px under it,
and the row under it is the bar's, drawn lower than it begins: the shadow can't run on into it,
and stopped dead after 5 px in the middle of the row. The app's stands with its foot `LINE_AIR`
over the sheets' line at its row's foot, reaching up into the row over it: a box of 43 px from
the row's top stood 6 px over the line. The one over New Workspace stands at its row's top, and
where the foot is one row, so that the row is a sheet's with the line at its foot, it's no
taller than keeps it 8 px over the line. The one over New Workspace is kept to its own row there, the status bar's where
the foot is two rows, since the row over it is a sheet's and isn't drawn where it is.

So that little is left over, a row's height is fitted to the window (`fitted`): of the
heights within a twentieth of what Settings asks, 47 to 53 px for 50, the one that leaves
least of the window's lines, and the nearest to what's asked among equals. Up to some 45 rows
that leaves 12 px at most where a fixed 50 left up to 49. A taller window can hold about as
many rows as a row has pixels, and then a pixel more or less on each changes nothing; so can
Comfortable rows in an ordinary window. What's left there is shared over and under the bar.
`CELLIX_ROW_FIT=0` keeps rows the height asked, which the tour needs to count its presses.

A message has the room of every detail it needs. The details give way from the far end, and a
message that still doesn't fit has the cursor's place too, which is back when the message
goes, at the next key. Until 0.10.0 a message kept 64 columns and no more: sharing the row,
what BibTeX said lost its end, the name of the file it couldn't open, to the cursor's place in
a window of 140 columns.

## Row height

`row_height` is `airy`, `comfortable` or `terminal`, chosen on Settings' Appearance page under
the theme. Airy is the default: a row 1.4 of the terminal's lines tall, which for a terminal
set to 14 pt is 26 pt for 13 pt words. Comfortable is 1.2, and `terminal` is the terminal's
own line, as every row was up to 0.9.2. It changes as it's clicked: everything is measured and
drawn again in a grid of as many rows as the window then holds. Where Cellix can't lay out
rows of its own (see "Not yet proven", item 6) the choice changes nothing, and a row is a
line. `CELLIX_ROW_HEIGHT` overrules the setting with a number of lines, 1 for the terminal's
own, which is how the harness chooses: the tour's terminal counts its size and its steps in
Cellix's rows (`TERM_ROW`), so a scene has the rows it always had.

On a taller row words and icons are the sizes they are on the terminal's line, in the middle
of the row. So is anything shorter than a row: a chip, a mark, a rule. What all but fills
its row on the terminal's line is a control with the row to itself (a button, a text box,
the band behind a picked row, a tip), and on a taller row those are one height, 0.78 of the
row: about 20 pt in a row of 26, as the look's seventh rule has it. The rule is the
rasterizer's (`stood`), so no control sets its own height for it.

A chip that holds words (a workspace's, a pill in the comments pane, what's under the pointer
in the status bar) is the design's 20 px in a row of 28: 0.714 of a taller row, by the same
rule, for anything drawn 0.7 to 0.8 of a row. What's shorter than that is a mark or a badge
and keeps its height.

Where a row has that room (`measures::roomy`: 1.3 lines or more, so Airy and not
Comfortable), words stand in the middle of what holds them, as they do in the design. On the
terminal's line a box or a chip is too short for a tail, and its words are raised a pixel or
two: `FIELD_LIFT`, the bar's `BAR_LIFT` and `WORD_LIFT`. None of those apply on a roomy row,
where they put a capital 6 px under a box's top and 13 over its foot (the user's second and
fourth points on trying 0.10.0 in Ghostty). A button's words stand by their font's span on
either (see Look). Three more follow from the same room:

- A card (the offer to install a language's server, the notice about logout, a stopped
  session's) has about a quarter of a row over its first row and under its last, as the
  design's has, set so its title's capitals are as far from its top as its buttons are from
  its foot (`CARD_EDGE`), and no empty row between its words and its buttons. The sheets'
  edges are drawn again under the two rows it only partly covers. On the terminal's line it keeps
  the half row and the gap, where a button all but fills its row.
- What's under the pointer in the status bar has one chip, a workspace's height, in the row's
  middle, with 0.6 of a column either side of what it holds: the branch, the count of
  problems, the bell, the button that quits. The bell and the power mark are 0.7 of the
  line, the design's size for an icon in the bar. On the terminal's line they're as they
  were, with the chip 8 px clear of the sheet over it.
- A button at the end of a pointed row paints its chip on the row's wash and leaves its cells
  the row's own ground. Given the wash's colour, they were a whole row of it behind a button
  that's 0.78 of one.
- That button stands inside the wash (`Grid::row_button`, `raster::inset`): it's the
  height it has on the terminal's line, 32 px, in a row of 50, and a row of a list that holds
  a button is washed its whole height when pointed at (Problems, Settings' language servers),
  so the button has 10 px of it over and 8 under. In a wash of 39 px set in from the row, its
  foot lay on the wash's once it went down onto the row's baseline (the polish review). A
  capital's height of its words stands in its middle, as every button's does, and it goes down
  with its words onto the baseline of the row's own words (`Grid::beside_words`,
  `WORDS_BASELINE`, a pixel on a row of 50): 10 px over it and 8 under in the row. A label in
  90 per cent of the row's size rode 3 px over the words beside it (0.11.0's release check, R2,
  and the polish review: Keep, Ask Agent, Keep as Tab), and 3 px down, Keep and Dismiss stood
  6 px over the next line in Review (the release review of next). Where its row's foot is an
  edge, the line under peek's header, a whole-row wash, a suggestion's row in Review, a button
  that would stand within 8 px of it stays in the row's middle instead
  (`measures::row_button_foot_lift`); on a row of 50 the pixel keeps 8. As tall
  as the wash, its edges lay on the wash's and the two read as one shape with a step in it; at
  27 px, for 6 px of wash, a "p"'s tail ended on its foot (0.11.0's release review: Keep, Keep
  as Tab, Show Source, Ask Agent, Install). Its corners are 6 px where a button's are 8 (`ROW_BUTTON_RADIUS`),
  nearer the wash's 4. Ask Agent on a problem's row and Install on a language server's are
  the two. Only on Airy rows: on Comfortable ones the wash is under 2 px taller than the
  button, which would be a rim of less than a pixel, and they stay the same height.
- A problem's message that doesn't fit ends in an ellipsis, two columns short of the button
  on a pointed row (`context::elide`): cut at whatever letter came a column before the
  button, "identifier 'c" read as what was said. The wash behind a pointed row of Settings is
  4 paint units inside its row at each end (`ROW_WASH`), so its middle is the row's: at
  0.055 of a row it rounded 0.4 px low, which a button inside it shows.

Appearance is taller than a short window, the more so on taller rows, so it scrolls: a part
that doesn't fit whole isn't drawn, and nothing is drawn over the bars under the page. Every
control keeps its turn under Tab, in view or not, and the page goes to the one the keyboard
is on.

## Look

The interface follows VS Code's layout, because that is what the target users already read
fluently: views, explorer, tabs, breadcrumbs, current-line highlight, indent guides.
The views (Explorer, Search, Outline, Source Control, Agent Context, and Settings at the end)
run across the top of the sidebar, level with the tabs, an icon every three columns, the one
in front with a line under it in the accent (the user's choice of 16 November 2026, after a
picture of Lapce; a rail down the left edge took five columns for them until then). A hairline runs under the row, the sidebar's width and level with the one under the tabs, between the views and what the one in front shows (asked for the same day). The side pane's tabs, and the icons at the top right after them, have the same line under them, so one line runs across the top of the window, broken by the frame's columns.

Under the views there's a row of air, and the heading of what the one in front shows
(the folder's name and its branch, OUTLINE, SOURCE CONTROL) is painted a third of a row up
into it (`SIDEBAR_RAISE`, 0.34): the same air between the line and its capitals as between it
and the capitals of the first row of its list. A view's tip, and the sidebar button's, stands
8 px under the views' line, in that air over the heading, where a row has room for it: on the
heading, it ended part way along its words and cut them ("QUEU", the branch's chip to "ma"). On a
row the terminal's line tall there's no such room, and it stands on the heading, covering it
whole. On the row straight under the line the heading read as part of the views (the user's
third point on trying 0.10.0). Only the heading's paint is raised
(`raise_sidebar_heading`): its cells, and every row under it, are where a press finds them,
and a press on the row of air counts as one on the heading (`on_sidebar_heading`). The
sidebar has a row less for its list. What a view says when it has nothing to list wraps
inside the sidebar, with two columns clear at its far edge.

A file's icon says its kind, as an icon theme's does: a disc with the letter cut out for Rust
and Python, Markdown's mark, git's diamond, braces for JSON, brackets for a config, angle
brackets for other code, and a page for the rest, each in its kind's colour. Folders are
filled, the same open or shut, since the chevron says which. The tree has a column for the
chevrons before the icons and a line down from each open folder; a file's tab has its icon
before its name. All of that is where there are pictures. In cells every file has the same
mark, a row starts a column sooner, and a tab has no icon.
 The view
itself starts a row down. With the sidebar closed the views go with it, and a button at the
start of the tabs brings both back; in Review it shows Source Control, which stands beside a
review. On a terminal without pictures an icon is block art two rows tall, so there the views
stay on a rail four columns wide down the left edge.

The sidebar has a button of its own, the side pane's mirrored (17 November 2026: it hid only by
Ctrl+B or a second press on the view in front, and nothing said so). It ends the views' row, after
Settings, in three columns as a view has: a window with its left part filled, the size of the
icons at the top right and on their chip when pointed at, named "Hide Sidebar" with Ctrl+B
quieter after it. Hidden, the button at the start of the tabs is the same window with its left
part in outline, "Show Sidebar". The narrowest sidebar is what holds every view with a column
clear at each end, 20 columns (`SIDEBAR_MIN_WIDTH`), and the button ends the views' row where the
sidebar has the 23 that hold it too (`SIDEBAR_WITH_BUTTON`), a window of 92 columns or more. In a
narrower one the sidebar keeps its 20 and the button stands at the start of the tabs, filled,
"Hide Sidebar", in the place where the button that brings it back stands once it's hidden
(decided after 0.11.0's review: a minimum of 23 for the button took a 72-column window's file
50 px, and the Task page's details were cut short for it). Without pictures the rail ends with it, ◧ under Settings at the rail's foot, and keeps it
as ◫ while the sidebar is hidden, since there the tabs have no button. Ctrl+B does the same.
Both themes are two surfaces: the frame (the views, the sidebar, the status bar) and the
sheets on it (the file and the side pane, whatever it shows, a terminal too, each with its
tabs). Cards, menus and dialogs are raised over those, and a text box is sunk. Text
has three weights of colour and there is one blue.

The dark theme is herdr's: the default dark look of its desktop app, the preset it calls
graphite, as herdr.dev shows it (the user's choice of 18 November 2026, "make the dark mode
color the same with herdr"; it had deep blue-grey surfaces of its own until then). The colours
are taken from herdr-app's `src/settings.rs`, where every role is worked out from graphite's
palette, and checked against the live mock on herdr.dev; herdr is Apache-2.0 and only its
colours are taken. The frame is herdr's chrome, #101114 (its sidebar, tab strip and status
bar: graphite's background a third of the way to black). A sheet is graphite's background,
#17191d, what herdr's panes stand on. What's raised, a menu, a card or a dialog, is herdr's
popover, #25272b (6% toward white), with a line round it. A page's code block is sunk to the
frame's depth and a text box a step under it, #0d0e11, as a box was before. Where herdr lays white over its ground, Cellix lays the same: a hover at
5.5%, a hairline at 9.4% and a scrollbar at 14%. What's picked is one colour everywhere, the
blue at 16% over its ground, as herdr.dev's picked rows are blue: a list's row on the sidebar or
a sheet (#263041 on the sheet), a palette's or a menu's (#323c4d), Settings' page, and the words
a comment is about on a page (`Theme::tint`). It was herdr's white at 9%, which was 1.10 to 1 from
a hover beside it and read as one; the review of 0.11.0 found both, and the workspace picker and
Settings already picking in blue. A picked row's words keep the text's bright colour on it,
Settings' page too, whose were the accent (on cells, with no wash, the accent still says it). A file listed twice in Agent Context, an instruction the
session also read, is picked only on the row it was opened from. A text box without the
keyboard has a line of white at 20% over a menu round it (`Theme::field_line`), 2.4 to 1 on the
frame: sunk a step under the frame, its fill is all but the frame's, and a menu's line round it
left an empty Replace box nearly gone; Source Control's message box has it too, where it had a
menu's, 1.7 to 1 on the sidebar. A menu is one colour wherever it opens: over a card, the
Comments pane's order menu or a link's, its line and shadow set it off the card, as in the light
theme (decided by the final review of 0.11.0, which found the order menu the one menu a step
lighter than the rest, #3d3f42 where every menu is #25272b). The emoji picker
drops from under the buttons that send what's being written, a row clear of them, onto the pane,
and is a menu's colour as any other: from its own button it covered the hint over them mid-word,
on the card. Text is
#eeeeef; the second weight is herdr's muted, #98999b, six tenths of the way from the sheet to
the text; the third is 0.55 of the way, #8d8e91, since herdr's dim (0.38, #696a6d) is 3.3 to 1
on the sheet and what's read here stays at 4.5 to 1. Dim is kept for what can't be done and
what git ignores. A selection is graphite's #394454. The blue is #76a8ff, the ring round a
focused box #96b2d7. Meaning is herdr's: the spinner in the blue while an agent works, #f0705f
when one needs you (herdr's blocked), green #4cc48c when one is done and for what's added,
graphite's yellow #d8be87 for what changed and its red #dc8c95 for what's removed or failed. A
warning in the problems list is the yellow, beside an error in the red: in #f0705f it read as
worse than the error (the light theme keeps its peach and red). A page's Important and Warning
callouts are graphite's magenta #b5a2ce and the yellow. Only the words at the foot that say an
agent needs you take #f0705f; the file's details at the bar's other end stay its quiet colour
(they took it too, which in a red read as errors). Matches Find isn't on are a wash of the
yellow, 28% over the sheet, 1.9 to 1 (at 18% they were 1.48 and hard to find), and the one it's
on is the blue at 35% with a ring of #96b2d7 round it where there are pixels
(`Theme::current_match`), in a file, on a page, in a terminal and in a list: in the selection's
colour it was 1.2 to 1 from the others. Light, the one it's on keeps the selection's blue and
takes the ring. Two selections, as Ctrl+D makes, are both selections and alike, as VS Code's
are. A diff's lines are washes of that green and of #f0705f, a fifth of the way over the
sheet and 0.36 for the words that changed: herdr's own washes, at 9%, read as grey beside a
review. Code
keeps Catppuccin Mocha's hues (mauve keywords, green strings, yellow types, peach numbers), each
7 to 1 or better on the new sheet, with functions in the blue and comments in the third
weight. A terminal in the side pane has graphite's sixteen colours, as herdr's panes have.

Where `settings.rs` and the site disagree, the code is followed, but in one place. The code
calls graphite's soft blue #96b2d7 the accent (it rings herdr's focused pane) and gives #76a8ff
to an agent that's done, with yellow #e5c07b while one works; the site paints the spinner and
its accents #76a8ff and a done agent green #4cc48c. Cellix's spinner is its one blue, and a done
dot in it would be the spinner's colour, so it takes the site's set, and #96b2d7 does here what
it does in herdr, ringing what has focus. herdr's canvas, #131518 in the 8 px between its
panes, has no surface of its own: the column between Cellix's sheets is the frame.

The light one is Catppuccin Latte's surfaces, unchanged by herdr's, the
editor on base, the sidebars on mantle, the frame on crust and menus white; its text and code
colours are Latte's hues deepened until each reads at 4.5 to 1, which Latte's own green,
yellow and blue don't (the end-to-end design the user approved on 2026-11-12, which replaced
"Latte's own colours for code" of 2026-11-09).
On a terminal, both themes move together so the frame sits on the terminal's own background and
its window padding shows no seam; a Latte terminal therefore shows the light theme a step paler
than Latte (decided with the user on 2026-11-09). So herdr's colours show as they are on a
terminal whose background is herdr's frame, #101114. On any other dark background the frame is
that background and every surface keeps its step from herdr's frame, the sheet 7, 8 and 9 above
it in red, green and blue and a menu 21 to 23: on #282c34 the frame is #282c34, the sheet
#2f343d and a menu #3d424b, and on black the sheet is #070809. The selection, Find's matches and
a diff's washes move with the sheet, or on most terminals an added line would be darker than
the sheet it's on. Text, the blue and the meaning colours stay as they are, but for the quiet
words: where their ground came up toward them they're brought toward the words' own colour
until they read as well as on herdr's (the third weight on #2f343d was 3.8 to 1). The light
theme does none of this and stays as Latte has it. It's all drawn to the grid, so it holds on the cell tier, which
writes every colour as 24-bit; a terminal that has only 256 colours maps them itself, and
graphite's surfaces, being nearly grey, land on separate steps of its grey ramp in their order
(the frame on 233, a sheet on 234, a menu on 235), where the blue-grey ones before them put the
frame and the panels on one. Activity icons span several cells:
shapes on the pixel tier and block art on cells, because a font glyph is too small beside a
real IDE. The highlight behind a selected or hovered icon on the rail keeps 8 px or more from
the icon on every side; it reaches into the rows either side, so the rail sits half a row lower,
its first icon a row from the top, which keeps the first one inside the window.

Sizes are VS Code's, measured in a terminal set to 14 pt, where a row is 18.5 pt (decided with
the user on 2026-11-15, who found everything too small; the Sizes board of the end-to-end
design). Words are seven tenths of a row, 13 pt, where they were three fifths, 11 pt beside 14 pt
code. Small words, a button's, a chip's and both bars', are nine tenths of that, and a section's
heading four fifths. A page's words are the interface's, so they grew with it. An icon beside a
name is 16 pt (`GLYPH`). An icon in a chip at the top right is 13 pt: VS Code's are 16 in a bar
35 pt tall, this bar is one row, and a larger icon would stand against its chip, which keeps 3 px
over and under the icon's box as VS Code's hover chips do. Every chip in a row of tabs, under
the pointer on those icons, the sidebar's button, the + or a tab behind, and the count of tabs
out of view, is one height (`measures::bar_chip`): on a row with room, 34 px of the terminal's
line, kept at that height, which leaves 8 px over and under it in a row of 50, and so 8 px over
the sheet's line under the row, and 4 px round a 13 pt icon; the icons' and the button's were a
control's 39 px, 5 px over that line, beside the count's 35 (0.11.0's release check, m4; the
8 px rule over the design's 20/28 chip, decided with the user). On a row the terminal's line
tall, nine tenths of it. Making the two tab rows two rows
tall, which would hold 16 pt, was weighed and left (2026-11-16, the user asking for the choice
to be made): it costs a row of the file in each pane for 3 pt, in a window that has forty rows
and inside Claude Code often fewer; at 13 pt the icons are the words' own size and read at a
glance; and a terminal without pictures would spend the row on nothing or be laid out
differently from one with them. A rail icon is 24 pt, on a rail five
columns wide with two rows from one icon to the next, either of which presses it; in cells the
rail keeps its four columns. Code is the terminal's own size and didn't change. Words a sixth
wider leave a name in the sidebar about five letters fewer before it's cut.

A row is 18.5 pt and words of 13 pt nearly fill it, so what's one row tall is sized for them,
and for their tails: words stand with their capitals in the middle of their row, and a "g"
reaches 4 px further down than an "h" reaches up. A text box is seven eighths of its row
(`FIELD_AIR`), the palette's nine tenths, and what's typed in it is raised 2 px (`FIELD_LIFT`),
which leaves a tail as far from the box's foot as a tall letter is from its top; the caret
stands in the box's middle, and what shares the box's line, a search box's switches and the
Find bar's count, is raised with the words. The band behind a picked or pointed row, a
palette's and a menu's, is 33 px of 36 (`BAND_HEIGHT`). A button's words stand by a capital's
height in its chip (`Grid::button_label`, `Paint::Text`'s `span_centred`): a capital's top as
far from its top as the baseline from its foot, in their font ("H"), whatever letters they
have, so every label of a size has one baseline, a row of buttons with it, and tails hang
under it as in any interface, 4 to 5 px over the foot of a button of 38. In the font's line,
as other words stand, "Show Source" had 5 px over its capitals and 10 under, and a card's
"Accept Rewrite" 9 over and 6 under its tails (0.11.0's release check, P2). Centred on its own
ink for a while, each label had its own baseline, and a row's rode high whenever one had a
"p": "Dismiss" stood 4 px under its button's top and 10 over its foot (0.11.0's polish
review); centred by a capital's top to a tail's foot, "Hg", every label without tails stood 2
to 3 px high and segmented controls 3 px over their row's words (the release review of next). A
button in a row of words, at the row's end or on its own, goes down with its words onto the
row's baseline (`Grid::beside_words`), the box moving and not the words: Reject and Accept
on a hunk's heading, Keep and Dismiss on a suggestion's row, Ask Agent, Install. A button by
itself is 38 px, one in a row 30 (32 nominal). A selection's buttons, the conflict header's,
the pane's order, segmented controls and Check My Review stand by it too. A workspace's chip is three quarters of the bar's row, keeps 8 px from the
window's edge where the terminal has no padding of its own, and its words and icons are
raised a pixel and a half in it; a pill in the comments pane is three quarters of a row too,
its words in its middle. An icon inside a text box or a chip is 13 pt (`SMALL_GLYPH`), an
arrow or a × in the Find bar the size of the ones beside names. A panel with a line round it
is drawn on whole pixels wherever it stands in its rows, so the line is one pixel of its own
colour on every side: between pixels it was two rows at half strength on one side and one at
full on the other. On a page, a heading two rows tall stands high enough in them to keep 7 px
over the line that follows, a paper's title over its subtitle. A line whose formula hangs under
it by more than a tail's depth, a fraction of sums, keeps an empty row under it, which the caret
doesn't stop on: it lay over the next line's words. In Agent Context a check has three rows
before it's cut, and a name is cut by what it measures, at the cell its room ends in; the
Timeline's heading is measured in its own small bold capitals before it's cut.

Air from lines (0.11.0's review). Nothing stands within 8 px of a line or a sheet's edge
(`measures::LINE_AIR`: 8 px of a Retina screen, 4 pt), measured from the line and not from a
row: on a row of 50 px a control 39 px tall has 5 px over it, and on the row under a tab row
that was 5 px from the tab row's line. So, where a row has room: a control on a row between
lines keeps the height it has on the terminal's line (a button at a pointed row's end, Show
Source under the tab row, the conflict header's buttons, the chip of the view showing in a
docked pane's header: a panel's `inset`; Show Source stands as far under the tab line as its end
is from the sheet's side, 15 px, and the scrollbar beside it starts `LINE_AIR` under its foot, where
its thumb stood 2 px under it); a tooltip under a bar stands under its control 8 px under
its row's top, where the line it hangs from is (10 px from the line's top), and ends 8 px from
the line down the window's side (`App::tip_end`). It's a layer, and covers the top of what's
under it, the branch's chip on the heading under the views or a minimap's first lines: moved
past them, as it was for a night, the rail's tip and Hide Sidebar's stood 85 px under their
icons and cut a folder's name in half, and a pane view's 60 px under its line (0.11.0's release
review). A pane's tab's tip, a cut name or an agent's context, stands from the tab's start,
inside the pane and 8 px from its lines, and is cut with "…" where it's longer: from the
pointer, the context's ran over the file's sheet's line and to 9 px from the window's side; a
menu that opens on the row under a tab row starts 8 px down it and
takes another row where it needs one (`Menu::hang`), and one dropped from a button stands on
the button's left edge 8 px under its foot (`Menu::under`), the Comments pane's order menu
too, in a menu's colour as every menu is; a submenu's first row is level with the row that
opened it, with every menu's air over and under its rows (`Menu::level`), and its panel stands
right against that menu's edge, its border touching the other's, and casts its shadow down and
away from it, not onto it: nothing of the menu, its border, its round corner or its ground, is
covered. With its shadow on the menu, its shadow darkened 10 px of the other (0.11.0's release
review, B7); 4 px over the menu's edge, it covered the menu's border and cut off its round corner,
and the two read as one shape (0.12.0's release review); a floating panel,
a menu, a palette or a tip, keeps both its sides 8 px or more from the sheets' upright lines,
moving a column along where it would stand on one (`measures::clear_of_lines`): Go to Symbol
and a tab's menu had their left line on the file's sheet's; the Find bar over the side pane
stands a row under the tab line, as it does over a file, ends where the pane's lists' words do,
two columns in, with its box from its edge where it has no chevron, and a sliver of what's
under it either side is cleared. A band behind a pointed or picked row stands 8 px in from the sheet's line on both
sides (`measures::band`), counting the half of the frame's column the side pane's sheet starts
in and the line's own two pixels (`measures::line_pixels`; counted as one, Problems' band
stood 7 px from it), in the sidebar's lists too (Source Control's, the Timeline's and Agent
Context's were 0.4 of a column in, 6 px); a comment's card is as far from that line on its right as on its left, a column, as every
card over the side pane is (`PANE_CARD_LEAD`, below); a program in the
side pane keeps the window's last column as air, as its first keeps the half column before the
sheet's edge. A floating panel's grid, the table picker's squares and the math panel's
buttons, stands as far from the panel's top as from its sides, and the table picker's label stands
on the squares' left edge, its capitals 24 px under them and its words 22 px over the panel's foot,
the panel no taller than that: it stood 15 px right of their edge, 59 px under them and 12 over
the foot. A task's box on a page is
square, at most two thirds of the line, in the room a list's marker has, with 9 px before its
words, which line up with every other item's (`rendered::MARKER`). A status message has the
bar up to where the details at its other end are painted, and an ellipsis only where that's
too short. Where a row is the terminal's own line there's no room for this air, and things
stand as they did.

One set of parts, everywhere. A button is one chip, nearly nine tenths of its row and inside it
(four fifths left its words 7 px under its top, which read as cramped): the
primary one filled with the accent, the rest a quiet fill, the least of them plain words (Edit and
Delete on a comment), and each takes a click anywhere it's painted, from one rectangle the painter
and the hit test share. Under the pointer a quiet one stands a second step off what's behind it,
lighter in the dark theme and deeper in the light one, on a card and in a dialog as in a pane
(`Theme::secondary`). A card that asks has its buttons at its right, ending where its words do,
one gap between each and the answer last and rightmost (`App::card_buttons_right`): Never, Not
Now, Install; Not Now, Copy Command, Turn On; Deny, Always Allow Here, Allow; Close, Start
Again. The install and logout cards had theirs from their words' left edge with the answer
first (0.11.0's polish review). Every action that isn't the main one is that quiet kind: Deny beside Allow,
a suggestion's Keep and Dismiss, Review with…, New Workspace and Resume at the foot, Cancel
(the design review of 0.11.0 found Deny and Dismiss as plain words, New Workspace in a picked
row's wash, and hovers in a menu's colour that went darker than the button at rest on a card). In
the dark theme the quiet fill is 20 steps over its ground, about herdr's raised, white at 9% over
graphite's sheet; 14 left Review with… dimmer than a picked row beside it. A row of choices,
Settings' and the New Worktree dialog's, is one track a button tall with a text box's line round
it (`Theme::field_line`; a menu's line was 1.34 to 1 on a dialog) and its ground showing through:
the chosen one is filled with the accent a pixel or two inside the line, the others are plain words, and
one pointed at takes the quiet fill. Each was a
box of the frame's colour, which read as more text boxes. A theme's card on Settings' Appearance
page is the theme as a tiny window, its frame, a sidebar in what's raised and a sheet: with the
sidebar in the frame's colour, as it really is, graphite's surfaces were 1.07 to 1 apart at that
size and the window showed no panels. One
measure sizes them all (`measures.rs`): a button's words keep a column
and a half either side inside its chip, whatever they are, and buttons side by side stand two
columns apart. In cells a button is its words between brackets. Text stands two columns inside
whatever holds it, a panel, a menu, a prompt; a card starts one column from its pane's
edge and is raised over it, as a menu is over the page. Beside the file, the side pane's sheet
starts half way into the pane's first column, so a card there, peek's or the one
asking to start an agent, has its panel start half way into its first column too
(`PANE_CARD_LEAD`), a column from the sheet's edge on both sides (it stood 6 px from it at the
left and 15 at the right); the pane's sheet shows in that half column, whatever stood there (a
terminal's cursor in it showed half of itself beside the card).
A comment's card keeps one inset on every side, `CARD_INSET` in `measures.rs`, 24 px: its words
start 24 px in from its left edge, what stands at its right end (a pill, Edit and Delete) ends 24
px from its right, its first words' capitals are 24 px under its top and its buttons' foot 24 px
over its own (`card_air`). A box inside it, as an agent's rewrite is, keeps `BOX_PADDING`, 16 px,
round its words and over the buttons under it. Its words stood 25 px in beside the file and 34
docked, the pill ended 33 px from the right and Delete 51, and a short card's foot was 44 px
under its last words where its top was 19 over its first. The cards' edges, and the list's head,
stand at the pane's text edge, where every view in the pane starts its words: the docked
Comments started at x27 where the Agents list did at x45.
A dialog's buttons, as a card's, stand on the row after its last control on a row with room,
and a row further down only on the terminal's line, where a button all but fills its row
(`card_gap`): New Worktree had two rows, 62 px, of nothing over its buttons. On a row with
room a dialog ends with its buttons' row, and its parts are raised in their rows, by paint,
so there's as much air over its title as under its buttons, about what its sides have (30 to
33 px beside 34), and 24 px between its last line or control and its buttons
(`measures::dialog_lifts`, `DIALOG_GAP`); a press lands where they're painted. With a row
empty over the title and one under the buttons it had 65 px over and 54 under, and its buttons
17 px under its words. A card in the window's corner, the
logout notice or the offer to install a language's server, and the cards over the side pane
(a stopped session's, an agent asking to start another), starts and ends inside its first and
last rows, and is a dialog in a corner: on a row with room its words are raised so 24 px stand
between them, or the box of a command, and its buttons, and its panel starts and stops so its
title's capitals and its buttons' foot have the air its sides have, 24 to 32 px
(`measures::card_parts`); they had 21 over and under and 17 or 11 over the buttons. On the
terminal's line a row stands between its words and its buttons. Its words wrap inside it, and
it stands inside one sheet, never across the line between the file and the pane
(`App::corner_frame`). Every floating card keeps 24 px of its ground inside its line on all four
sides, 26 from its edge with the line and the blended pixel inside it (`App::card_room`): its
panel half a column into its first column (`App::card_lead`), so its words' ink is 24 px in,
its buttons painted right of their cells so the last ends as far from its right
(`App::card_button_nudge`), its title's capitals 24 px under its top and its buttons' foot 24
over its foot, and a command's box on those insets, ending where the buttons do, with the
command 12 px into it (`COMMAND_PADDING`), as the Ask card's question's field is, with what's
typed 12 px into it: a column into the card's cells it stood 20 px from its line (the release
review of 0.12.0, q1-ask-field). The agent's name over an answer ends there too, where two
columns in it ended 37 px inside the line (q4-ask-follow-up). The install card over the file had 32 px and 29, and
over the pane the buttons ended 32 to 34 px from the right; 12 px outside the words, the box ran
22 px past them (the release review of next), and 24 px from the edge, the buttons kept 22 of
ground. Over the side pane every card is the asking
card's: a column in from each of the pane's sides however wide it is, so it stands about 16 px
from the pane's lines. The asking card's mark beside its title stands taller than the title's
capitals, so that row goes down 4 px.
Two columns in and capped at its width, the install, logout and stopped
cards stood 24 px from the pane's left and 33 from its right with their words 33 px in
(0.11.0's release review). A card in a sheet's corner ends on the sheet's last row and is
painted part of a row lower, so its panel's foot stands as far over the line at the sheet's
foot as its side stands from the sheet's side: 16 px over the side pane, about 32 over the
file (`App::card_drop`), where a row has room; a press lands where its buttons are painted. They ended a row higher
and floated 79 px over the pane's foot (0.11.0's release check, P6). The rows its round ends are on
keep the colour of whatever each cell lies on, since the corner can be over the file, the side
pane and the line between them, and that colour is painted over them, so a page's words under
the card don't show their tops over it. Its panel's top and bottom are moved to the nearest whole
pixel, which keeps those two lines as sharp as its sides'. A bar that holds a text box, the Find bar
and a prompt, keeps a quarter of a row clear above and below its rows, which only paint can show,
and a box's text starts a column inside it; a prompt's question and box are raised so its box's
foot is as far from the bar's as the question's capitals are from its top (they were 10 px and
19). What a box holds stands in its middle the same way: the symbols panel's last line as far
from its foot as its buttons from its top, peek's card as far from the pane's foot as from its
sides (reaching part way into the pane's last row, `Grid::exact_fill`, since a panel shorter
than a row is stood as a control), a chip's mark and words with the same air either side (the
branch's, New Workspace's "+"). A section's heading is small, bold, quiet capitals
wherever it is. Nothing is as wide as its panel for its own sake: Commit and Sync are as wide as
their words, from the panel's text edge, where Commit had filled the sidebar. In a list of many
rows an action is words in the accent until its row is pointed at, then a button: Install,
beside the language servers. Every list in the side pane starts a row under its tabs, and the
pane's first tab starts flush with the pane, as a file's does with its sheet. Settings has one box that looks through every page, where only its Keyboard page
had a search of its own: what fits is listed in one column with each row's page named beside
it, the pages at the left count what was found on them, and a row works there as it does on
its own page, a switch, a choice or a command's keys. These came from the end-to-end design (canvas PLJ1YE4pkaPSHEfKwBv42N, version 7)
and the measurements taken of every surface before it was applied: buttons had from 1.4 to 3.6
columns of room and stood one apart, and a menu's text stood one column in while a dialog's
stood two. Every sidebar is one width,
Review's list too, with the same title row: its name in small quiet capitals, two columns in.
A tab is one shape, in the editor and in the pane: the strip is the frame's colour, and the
tab in front takes the colour of what's under it, the sheet or the pane, with the accent's
line along its top. One behind is its name, brighter under the pointer. A menu open takes the
pointer: nothing under it, or beside it, is lit while it's open, where the Agents list's row
under the + menu lit as the pointer crossed the menu; the menu's own rows go by where the
pointer is over it. A menu that drops from under a line, a tab's and the pane's +'s, stands
8 px clear of the line's far side (a line is two pixels thick on a Retina screen; counted as
one, a tab's menu stood 7 px from it and the +'s on it), and one from a button 8 px clear of
the button's foot (`Menu::hang_under`); with no rules to take what whole rows leave over, a
hung menu keeps the rows it would have had and the drop comes out of its air, where another
whole row shared over and under gave Review with… 56 px to its words against every other
menu's 40. Overlays sit on row lines: a menu
is whole rows, with half a row of air over its first row and under its last in every menu and
half a row at least for each rule, its hover band clear of them; where whole rows leave half a
row over, which an odd number of rules does, the rules share it, never the edges (they had a
quarter of a row or half by how many rules there were, 26 px from the edge to the words in one
menu and 39 in the next). A row that can't be picked and says why, an agent not installed, has
its name and its note in one colour, readable and quieter than a row that can be picked
(`Theme::unavailable`): the quiet words' in the dark theme, Latte's subtext0, #6c6f85, in the
light one, whose quiet words are all but its text's. Dimmed past its note, the name read as less
than "not installed". One that can't be picked and says nothing is dimmed. A menu that marks the one in use, the Comments pane's order, has its check in
a column of its own, as an icon is, so every row's words start in one place. It drops from the bottom edge of what opened it; a tooltip is one row, on
the row under its icon. A tip longer than the room it has, as an agent's tab's on what its
context holds, goes on to a second line, the terminal's line under the first, broken at a " · "
where one leaves the first line within the room: cut on one line it lost what it said last,
"right-click to open the tra…", with 48 px empty after it (0.11.0's release check, P7). Broken
there, the second line is a phrase of its own and starts with a capital, "Right-click to open the
transcript" (the release review of 0.12.0, 64-transcript-tab-hover). While a
tip's box is its words' ink, ascenders, brackets and tails (`measures::TIP_INK`), with
`LINE_AIR` and its line over and under it and a pixel for the ink's half pixel under the row's
middle (`measures::tip_box`): 43 px of a row of 50, drawn exactly there (`Grid::exact_popup`),
where a panel shorter than its row is otherwise stood as a control, 39 px, which held the words 7
px under its line and 6 over its foot (the release review of 0.12.0, 64, 166, 13-quit-hover,
105). While a
tip covers the side pane's terminal's cursor, the cursor goes: a tip covers the row under its
line but the top 10 px of it, and a sliver of the cursor stood on the tip's edge (R4). Nothing
covers part of a row. A tip's corners are round over plain ground: where a letter of the cells
is under one and nothing painted covers it there, that corner of the square the tip stands in
is filled with the cell's ground first, since a pixel of the prompt's "$" stood at the corner
of the tip under a cut tab (166). A colour under a corner still shows past it. The side pane's
program starts on the row under its tabs, whose line is that row's top pixels, and a cell is a
whole row tall, so its cursor stood on the line (58, T8). Where a row has room, the first row's
top is painted the pane's ground `LINE_AIR` past the line, short of its words' capitals: 10 px
of a row of 50, so a cursor or a coloured cell there stands 8 px under the line, a little
shorter than on the rows under it. The cells don't move, so rows, presses and the cell tier
are as they were, and on the terminal's own line there's no air to give. A palette (Quick Open, the commands, and
every list that asks with a field) is whole rows too: the field's row, one row of air, and its
list. The field is a box, as the Find bar's and a prompt's are, nine tenths of its row as the
canvas draws a field, a column in from the panel's edges where the picked row's band stands
too. What's typed starts where a row's words start: in a list whose rows have icons (files,
symbols, folders) the field has the search icon over theirs and its words after it, and in a
list of plain rows (the commands, branches, tasks) it has no icon and its words stand over
theirs, as VS Code's do. The hint starts where typing will, with the caret just before it,
and the caret stands just after the last letter typed. A box is sunk into what holds it: where
the text box's colour is the panel's own, as both are white in the light theme, the box is a
step from the panel toward the words' colour (`Theme::popup_input`). It took the sheet's colour
until a light terminal's pictures showed the sheet moved to white as well. The row of air is shared out so that the box stands as far under the
panel's top as the first row's band stands under the box and the panel's foot under the last
row's band, 0.43 of a row each: the field is painted a third of a row down its own row and the
list as much up theirs, which only paint can show, and a press lands on the row painted there.
In cells the field has the first row, the list starts on the third, and the picked row's colour
is in its cells. It was a field with no box and a line under it, 18 px from the panel's top and
30 from the line, which read as off-centre (the user, 2026-11-14), with the list hanging from
the line and nothing under its last row. The list keeps a row when nothing matches, which says
so, so the field never moves as the list empties. A list longer than its twelve rows scrolls:
it follows the picked row, a group's name or rule coming with the first row under it, Page Up
and Page Down move a screen less a row, and the wheel moves it two rows a notch wherever the
pointer is, leaving the pick where it was. A thumb 4 px wide in the middle of the column between
the band and the panel's edge, lighter than the edge's line, shows how much there is and how far
down this is; in cells it's half blocks in the panel's last column. Rows that arrive while a big folder is still being walked don't move what's
being looked at. The field is a text box like the others, a `field::TextBox`, which a comment's box
is too, a press puts the caret at the nearer edge of the letter pressed, and a query longer than
the box shows the part the caret is in. A press anywhere in the panel that isn't on a row to pick
leaves it open.

Every box is one editing core, the file's own: a `field::TextBox` is a `cellix_core::Buffer` and
its `Selections`, so a box and the file agree on what a letter, a word, a line and a selection
are, and move through one table (`field::motion`, which `Editor::run` uses too). The comment
composer and a reply, Ask's field, the commit message, the palette's field, the emoji picker's,
a prompt's (Rename tab, Rename Symbol, Go to Line, a file's name, a stash's message, Save Review
and the rest), the Find bar's two, the Search sidebar's two, Settings' filter and the New
Worktree dialog's all are one. Until 23 November 2026 a box was a `String` and a caret, with no
selection "by design" (the user: "I cannot do shift scan with arrows … We need the same writing
capabilities everywhere in the app"). Now every key does in a box what it does in the file, in
each form a terminal sends it: Shift with each move selects (a letter, a word with Option or Ctrl
as `ESC b`/`ESC f` or `CSI 1;3D`, the line's ends with Home and End or Cmd+Left and Right as
Ghostty sends them, the text's ends with Ctrl+Home and End, a line up or down in a box of
several), Ctrl+A selects all the box's words (it went to the line's start, as a Mac field's
does), what's typed or pasted replaces what's selected and Backspace and Delete take it, Ctrl+Z
and Ctrl+Shift+Z or Ctrl+Y undo and redo as the file groups them, and Up and Down keep the
column they set out from. Copy, Cut and Paste are the app's, which has the clipboard
(`App::box_clipboard`): with something selected in the box that has the keyboard
(`App::focused_box`), Ctrl+C and Ctrl+Shift+C copy it and Ctrl+X cuts it as Delete would, so the
box does what it does after any edit; with nothing selected they do what they did. Enter, Esc
and Tab are still each box's own. The selection is drawn as the file draws its own, the
selection's colour behind the words (`palette::box_selection`), and the caret where it was made
to. A table test sends the same keys to a one-line file and to each kind of box and compares
words, caret and selection (`every_key_does_in_every_box_what_it_does_in_the_file`).

The mouse in a box is the file's too (`App::box_mouse`): a press puts the caret at the nearer
edge of the letter painted under the pointer, a drag selects from it, a second press selects the
word and a third the line, all of a box of one line, and Shift with a press selects from the
caret. Each box says which byte a place is (`App::box_place`: `palette::input_press` for the boxes
`palette::input` draws, and the palette's, Settings', the commit message's, the emoji picker's,
the comment's (`threads::input_place`) and Ask's own), and a drag that leaves the box goes to the
nearest place in it. Until 23 November 2026 the comment's box and Ask's took no press at all.
With something selected the caret stands on the selection's edge (`palette::caret_nudge_in`, with `selected`).

A box of one line gives its words afresh with its caret as far from their end as it stood
(`TextBox::set`), so one opened with words in it, Rename tab with the tab's name, has its caret
at their end; Up and Down aren't its own (`TextBox::line_key`), they step through matches or
move what it's over. What's pasted goes in at the caret, and a press puts the caret at the
nearer edge of the letter pressed
(`palette::input_press`). `palette::input` draws the caret where it stands, and a text longer
than its box shows the part the caret is in (`hidden_before`, shared with the palette's field),
its words cut a column short of the box's far edge. Painted, the caret keeps clear of the
letters either side of it (`palette::caret_nudge_in`, which the palette's field and Settings' filter
share): just past the ink of the letter it follows where a space or the end comes next, in the
middle of a space it follows, and centred on the edge between two letters, the only room there
is. Placed 0.1 of a column before the end of the letter it followed, it stood 1 px from a "p"
after it and ran into an "r"'s arm before it at the end ("orden", the ui-expert's review). In
cells the caret is in the cell typing goes into. A box being written in takes Ctrl+W, as a shell and the
comment composer do, for the word before the caret, and the tab under it stays: until 20
November 2026 Ctrl+W closed that tab from the palette, a prompt, Find and the Search box. Ctrl+Q
quits from anywhere. Settings' filter is the exception, since typing anywhere in Settings goes
there without its having the keyboard: the deletes edit it whatever has the keyboard, the arrows
and Home and End move along it only while no control has the keyboard, and Ctrl+W closes
Settings as it closes any tab (the user's word). The emoji picker's plain arrows move among
the emoji, so its field takes every other key a box takes (Shift and Option with the arrows,
Home and End, the deletes, undo). The commit message wraps in its box as a
comment does (`threads::wrap_spans`), spaces kept where they were typed, and shows the rows up to
the caret's.

Every row to pick is drawn one way (`palette::Parts`): an icon where its kind has one, its
words, something quieter after them at 85 percent, and something at its far end, which the
words stop two columns short of. What's set smaller is dropped a pixel to stand on the words'
baseline, and on the picked row's band it's halfway between the quiet colour and the words',
since the quiet colour is too faint there and the words' own made a folder as loud as its name. A file is its icon in the colour the tree gives its type, its
name, then the folder it's in, as VS Code lists one: the name is what's looked for and a long
path used to lose it. A path too long for its room keeps where it starts from, `~` or the
root, and its end, and loses whole folders after its start behind a "…", `~/…/parent/name`,
the cut always at its left, here, in the welcome page's Recent and in the list of workspaces
running here (`palette::path_cut`); past that its name loses its start. Cut by letters in its
middle, the "…" fell somewhere different in every row and through a folder's name. A symbol is its kind's
icon, its name, what it sits in, and its group or file; a command its name and its keys; a
task its name and what it runs. The row's icon and words stand under the field's own. The
letters that were typed are picked out in the accent and bold (`picked_out`): where they stand
together, at the start of a word for choice, there; otherwise in order, from the end of a path
so the name is tried before the folders. That's a second reading of the match, for the eye,
and the ranking is still `fuzzy_score`'s. The picked row of a file says the place typed after
its name ("line 120"), which is where Enter opens it.

Before anything is typed, Quick Open lists the files last looked at, up to eight, under
RECENTLY OPENED, and the folder's own under FILES: those brought to the front in this run, the
last first, then the ones the folder had open before (`recent.json`), without the file in view.
The one before this is the first row and so the picked one, which makes Ctrl+P and Enter the way
back and forth between two files, as it is in VS Code. The walk leaves those files out of what
it lists after, so each is there once. Typed, the groups go and the ranking is the match's, with
a recent file ahead of another that matches as well, being earlier in the list.

The command palette does the same with commands: the six last run from it under RECENTLY USED,
the last at the top, and every other command under OTHER COMMANDS by name, as VS Code has them.
Only a command picked in the palette counts, since one with keys that are used never needs
the list. They're kept in the folder's `state.json` with the tabs. With none run yet the list is
every command by name, with no groups. A command's title is its category and its name, `Git:
Stash Changes`, as VS Code's are, so by name a category's commands stand together: Agent,
Comments, Document, Edit, File, Format, Git, Help, Merge Conflict, Navigate, Notebook,
Preferences, Search, Tasks, Terminal, Track Changes, View, Workspace. The Keyboard page lists
them under the same categories, each a heading in small capitals with a rule after it, as a
group in the palette is headed, with its commands under it by name alone and an empty row
before the next: a title in every row left a fifth of them shortened in a 120-column window on
a terminal without pictures, and a name alone has the column to itself. Scrolled into a
category, its heading stays on the list's first row. In cells the rule is a run of dashes.
What the page's filter finds has no heading over it, so it keeps its whole title. What's typed is looked for in the name first and scored as it was before
there were categories, so the letters a category happens to share don't rank a command. A
category typed before a space narrows to it: `git st` is Git's commands with "st" in their
names and `git ` is all of them, in the list's order. Failing both, the letters anywhere in the
title. A command loses nothing for a long name, as a file does for a long path, so commands
that match alike stay in the list's order and a category's stay together. The name alone is what settings and a folder's state
knew a command by, and still means it (`palette::goes_by`); keys set on the Keyboard page are
written under the title and the old entry goes. The list holds the commands that have something to act on now (`App::can_run`, in
`app/available.rs`), as VS Code's does by its `when` clauses: each is the test the command's own
handler makes before it does anything, or the one a menu greys it by. A command is left out
only when what it needs is known to be missing. What can't be told without reading the disk
again stays and says what's missing when run: a project's tasks, the folders opened lately, an
agent's turns while Review is closed. Whether there's a repository, a merge under way and
anything stashed are asked of git with the rest of the look at the disk every couple of
seconds, and until the first answer the commands that turn on them are listed. Keys run a
command whether it's listed or not. `>` typed first in Quick Open is the same list, the
sign staying in the field as `@` and `#` do for symbols, and taking it away lists the files
again. The hint names all three and `:line`, in as few words as the field's cells need.

Quick Open reads a path the way it's copied, since the file a person wants is most often one
an agent or a compiler has just named. What's pasted is cleaned of what it was copied with
(`palette::pasted_path`: quotes, backticks, brackets, a `file://`, an agent's `@`, a full stop, a
compiler's message after the place, and the words round it when it's a sentence that names a
file). What's typed or pasted is then split into a path and a place (`path_and_place`: `:120`,
`:120:8`, `:120-140`, `#L120`, `(120,8)`), and the app reads the path from the workspace's
folder, from the top of the disk, from `~` or after a diff's `a/` and `b/` (`App::file_named`). The
list is matched against the path as the list writes paths, and a path that names a file that's
there puts that file first, whether the walk found it or not: the walk leaves out what
`.gitignore` names, can't reach outside the folder, and in a big tree may not have got there.
Enter opens the picked file at the place. VS Code does the same with `file:line` and with whole
paths. A list or box that's open has the keyboard, so it takes what's pasted, by the terminal
or by Ctrl+V, before anything written under it does. Ctrl+P is one of the few keys that leave
the side pane's program (with the commands, Find, the pane's own keys and Quit), as it's one
VS Code keeps from its terminal: the file an agent names is wanted while the keyboard is still
in its chat. A file picked from Quick Open takes the keyboard.

Git has two places. Source Control in the sidebar decides what goes in: a message, staged and
unstaged files, Commit, and Push or Pull, as in VS Code. Review reads what changed, hunk by hunk,
and takes over the explorer's width unless Source Control is showing, in which case it stands
beside the panel with no list of its own.

Cellix often lives in a pane beside an agent, so width decides the layout. Under 70
columns the activity bar goes, the sidebar becomes a drawer over the code, splits collapse
to the focused pane, the status bar keeps only what fits, and review stacks changes.
Under 80 columns, about what Claude Code leaves Cellix beside its chat, the layout is made
for that width rather than squeezed. Between 70 and 119 columns with the side pane open, the
sidebar stands aside so the file and the pane keep their room; a view opened from the
palette, the rail or the tabs' button comes as a drawer over the file, stopping at the pane's
edge, and closes on a press elsewhere, when the pane closes, or when the window has room
again. The five panel icons lead the side pane's header, as
VS Code's panel shows its views, with the one showing named and no tab of its own. The top
right keeps one icon that shows or hides the pane. Tabs out of view are
counted at the ends of the strip ("‹ 2 more", "3 more ›"), and each count lists its tabs. A
count is a chip with 14 px of air either side of its words, and the one at the strip's start
stands 8 px in from the sheet's edge (it stood 3 px from it, with 24 px after its words). The wheel over the file's strip of tabs moves it sideways, a tab a notch, when they don't all
fit, as VS Code's does, with the counts at its ends following; the tab in front stays in
front and may go out of view, until another is chosen, which brings the strip to it. With
every tab in view the wheel there does nothing. The side pane keeps every tab in view, cut
short as it must, so there the wheel only steps from tab to tab where the tab in front shows
alone with its chevron, as moving the strip would bring the next into view.
Where there are pictures, every tab, a file's and the side pane's, has one anatomy, measured
to the ink (`measures::TAB_MARK` and its neighbours): about 17 px before its mark, 8 px from the
mark to its name, 12 px from its name, or from what its agent is doing, to its ×, and as much
after the × as before the mark, whatever the tab shows (0.11.0's final review, P12: a file's
tab was 21 | icon | name | 51 to its × | 13, and the air after a pane tab's × ran from 9 px on
a working tab to 25 on an idle one, since its slack was shared as the tab stood with nothing to
say). Each tab is as wide as what it paints: the lead, its mark, its name, how full its agent's
context is, what it's doing half a cell (8 px) after that, and 12 px, its × and its air; its
columns round away up to 5 px of that air, so a sliver doesn't cost a neighbour a column, and
what its whole columns hold past what it paints is shared in thirds, before the mark, before the
× and after it, so each is 17 to 21 px (halved at its ends, the air before a mark ran from 16 px
to 24 by tab); while it shows what its agent is doing it has two whole columns more for that,
so its share, and its name, don't move as its agent starts and stops. A file's tabs abut, as
the pane's do, where a column stood between them and an unselected tab's name stood 110 px
from the next one's icon. The Settings page's tab has the mark Settings has on the rail. A tab
behind keeps its ×'s room, as VS Code's do, so a tab doesn't move as it's pointed at, and two
tabs behind the one in front, a file's or the pane's, whole or cut short, have a hairline
between them, so that room reads as the tab's own (the tab in front is its own shape). Between
two files' tabs the line stands in the middle of the air between what the first paints, its
name while its × shows only under the pointer, and the next one's mark, and not beside a tab
under the pointer, whose × and its chip stand there: at their edge it stood 55 px after a name
and 16 px before the next mark (0.11.0's release review, m4). It's measured to the name's last
letter's ink, which its measure runs about 3 px past (`measures::WORDS_RUN_ON`): from the
measure it stood 37 px after README.md and 32 before PLAN.md (0.11.0's release check). A tab
behind keeps its ×'s room, so the line stands about 34 px from either, and couldn't be nearer
without the tab growing under the pointer (decided with the user, 2026-11-19). A tab dragged
along the row has its bar where that line would stand, and before the first tab in the middle
of the air between the sheet's line and its mark: at the strip's start it stood on the sheet's
line, which read as the line lit (R5, 97). The chip under the pointer on a
file tab's × is about 30 by 28 px round the ×'s ink, kept at that height on a taller row, 8 px
and more over the line under the tab in front; grown to the row's chip it was 37 px, its foot 3
px over that line. Counted in letters, as cells count them, a tab
was nearly twice what it painted, and two tabs, an agent's and a terminal's, were enough to
cut the row short with 200 px to spare, which took the + away (found in 0.11.0's tour). So
the + stands a cell after the last tab whenever the tabs and it fit, and its menu is in reach;
it gives way only when the row truly overflows. With the button that maximizes the pane right
after it, the + stands in the middle of the air from the last tab's ink to the button's, by
what's drawn: the ×'s of the tab in front, and what a tab behind draws, whose × shows only
under the pointer: at the start of its columns it stood 39 px after a tab's × and 34 before the
button (0.11.0's release check, m1-agent-tabs), and by the room a tab behind keeps for its ×,
79 px after its spinner and 47 before the button (the release review of 0.12.0, 172). A name is never cut mid-letter: it's whole
while the tab has the width it paints, and only short of that ends in a "…" at its measured
width. A tab in front, or under the pointer, has its words in the text's own colour, whatever
its program is doing; one whose program ended says so by its mark, dimmed. Behind, they're the
tabs' quieter words; in the light theme a step quieter than its other quiet words, #686b81
against the front's #4c4f69, as VS Code's light theme has them, where they were 1.1 to 1 from
the front's and a tab behind read as the one in front (0.11.0's release check). The screen's text,
which the tour and the tests read, holds as much of a painted tab's name as its columns do: its
× in its last column, what it's doing and its context before that. On cells a tab is its
letters and the columns round them, as before.
A side pane whose tabs don't all fit before the icons at the top right keeps every tab in
the row. The one in front, a tab in the shape every tab has, hugs its measured name and its ×, with none
of the slack a tab has when there's room, since that slack is what would cut its neighbours.
Where the marks of those behind leave it too little room, its own name is cut short too, with a
"…" and its " (2)" kept, down to three letters, and its tip names it whole; what its columns
hold past the cut name is shared by its two ends, so they keep a whole tab's air. Kept whole, a name
taken from what was asked ("Fix the login redirect… (2)") sent both tabs behind it and the +
into the chevron's list with 141 px empty, and "Claude Code" beside a terminal left no room for
the button that maximizes the pane (the release review of 0.12.0, m1-agent-names, w0-infra).
Those behind it stand as names alone, half a column of air either side, whole while they
fit and then cut short with a "…", sharing the room evenly as VS Code's shrink mode does: each
shows the same number of letters, as many as fit; one whose whole name is shorter keeps it and
leaves the rest to the others; and what whole columns leave over goes a column at a time to the
tabs cut short, as air after their names where it would hold a letter one has and the others
haven't, so no room stands empty after the + while a name is cut. A name cut short keeps three
letters at the least, or two where the third is a space; one or two letters and a "…" ("p…",
"pr…") said nothing its mark doesn't (the release review of 0.12.0, 14-agents, m1-agent-tabs).
No tab is at its mark while another shows any of its name: where three letters each have no
room, they go to their marks together (the user's choice, 2026-11-19; the widest gave a column
at a time, and Gemini CLI went to its mark while Claude Code kept "pricing p…", 0.11.0's release
check, R3, and then, one going to its mark first, "pri…" with the room it freed after the +). Two of
them side by side have a hairline between them, in the middle of the air from the ink of one
to the ink of the next, as between two whole tabs behind, so what
one is doing at its end is never read as the next one's. Its mark and its name stand where
they do while it's in front, so neither moves as it's brought forward: cut short, the first
tab's mark stood 5 px nearer the pane's edge than in front (0.11.0's release check, A7). A
spinner stands about 16 px from its end: at 10 and 11 px either side of that line, two read
as one (0.11.0's release review, m1-agent-tabs). A dot follows the name as on a whole tab,
in the middle of the room a spinner would have there, about 14 px to its ink, and what the
tab's last column has to spare is after it: at the tab's end, the spare room stood
between the name and the dot, 33 px, and the dot read as the next tab's (0.11.0's release
check, m1-agent-tab-done). Short of three letters and the "…" each, the
tabs behind are cut to their marks, and what their programs are doing beside them: the mark in the tab's
middle with the same air either side, about 16 px to its ink, in three columns, or five with
its state, and under the pointer a chip that's exactly the mark and that air (it kept a chip
five columns wide with the mark at its left, 16 px before it and 47 after). Side by side, tabs
at their marks have the same air from one's ink to the next one's all along them, and half of
it at either end, so the line between two stands as far from both on every one: each centred in
its own whole columns, the air beside the lines ran 14 to 21 px, by how each one's columns
rounded (the release review of 0.12.0, m2-agents-urgency). The first tab in
the row has the sheet's line just before it, and takes what it needs, a column more, for its
mark to stand in the middle between that line and its end with its chip 8 px from the line
(the user's choice, 2026-11-19: in three columns it read 22 | mark | 14 from the line, and
centred its chip would have come 4 px from it; 0.11.0's release review, A4), where the row
has that column; where it hasn't, every tab staying in the row comes first, and the mark
stands in the middle of its three (three agents in the tour's narrow window). Every tab has a
mark, and marks tell tabs apart, as herdr's narrow tabs do; letters come back, to every name
alike, as room allows. On cells a view's tab, a name elsewhere, is cut to the
letter its icon has at the top right. The + keeps its place after the tabs while any tab is
cut rather than listed, and the button that maximizes the pane its three columns, a column
after the + and no further than the end of the pane's part of the row: the names behind are cut
to their marks, and the one in front to three letters, before either gives way. Where tabs at their marks leave room, it falls after the
button, so the + stays in the middle between the last tab's ink and the button's: kept at the
row's end, the button stood 70 px after the + against 39 before it (the release review of
0.12.0, m2-agents-urgency, 71-sample-changes-row-narrow). With
the button gone, the +'s own last column of air may be the column kept clear before the icons
at the top right, which is air too: three agents in a pane of 44 columns were a column short,
and showed the tab in front alone with 154 px empty (0.11.0's release review,
m1-agent-tabs-narrow). The
+ gave way whenever the names had less than three letters each, which left a row of agents
with nothing to start another from beside it (0.11.0's final review: 167, 131,
m1-agent-tabs-narrow, 173-peek-narrow), and the button with it, though inside Claude Code,
where Cellix has at most 45% of the window and its side pane opens 40 columns wide, the user
had no other way in sight to give the pane the window (decided on 2026-11-16). The button
goes in one case only, a row whose tabs fit, those behind at their marks and the one in front at
three letters, with the + only without it, since every tab staying in sight comes first; the
columns it leaves go back to the name in front, and a double click on a tab maximizes there as
anywhere.
A tab behind has no ×, so a click anywhere on it brings it forward, and a cut name is a hover
away. Only when even the marks, the name in front at three letters and the + don't fit does
the row show the tab in front alone,
with a chevron that lists every tab and what + would start. The icons stand three columns
apart on their row's own ground, and nothing runs under them.
A program's tab in the side pane has its mark before its name, where a file's tab has its icon
and at its size: Cellix's own marks, never a vendor's logo. Claude Code's is an eight-line
burst in a warm clay, Codex's a hexagon with a prompt inside it in the text's colour, Gemini
CLI's a filled star of four points in a blue of its own. OpenCode, Copilot CLI and Cursor's
agent, which have none of Cellix's, are a filled square in the quieter text colour with
their initial cut out of it, O and C, and Cursor's a pointer cut out of it (it shared Copilot
CLI's C, and the two read as one). Kiro CLI's is the same square with a K cut out of it, as
tall and wide as OpenCode's O, in a violet of its own (#b192f2 on dark, #7f55d6 on light, each
as strong against its ground as Claude Code's clay), not
Kiro's ghost; any other program, one that reports by OSC 7501 under a
name of its own, the same square with its own name's initial; a task's tab has a shell's
prompt, `>_`. One meaning to a mark: a hollow square stood for an agent not installed, a
running Copilot CLI and an ended task at once, and read as a box to tick (0.11.0's review).
On cells one letter each, `✻ ⬡ ✦`, the initial for the squares (Kiro CLI's `K`), `↖` for Cursor's and `>` for
a prompt. In the + menu, an agent that isn't installed keeps its own mark, dimmed; Review
with… lists only those installed. Every tab has a mark: a shell's has its prompt at its prompt as while a command runs
in it, the browser's its globe, and where there are pictures each view's the one its icon
has, the Agents list's, Comments' bubble, the inbox's bell, the Preview's picture, Problems'
circled ×, the Log's lines (a shell at its prompt and the inbox were names alone beside
tabs with marks). A program that says what it is (OSC 7501's `app`) names its tab, as it
names its row in the Agents list and the inbox, until it takes that back: the tab said
"Terminal" beside a row that said "deploy". A shell running a command is named after the
command with a shell's mark before it (below). An agent's tab shows what its agent is doing half a cell after its name: the spinner in
the accent while it works, a dot in the attention colour while it asks, a green dot when it
finished a turn while its tab wasn't in front, until it is, and nothing while it's idle. A dot
stands in the middle of the room a spinner has, about 14 px after the name and 19 to 24 before
the ×: at the room's start, the needs-you dot stood 24 to 28 px from the × against a spinner's
19 (0.11.0's release check, m1-agent-tab-asking, 170). An
idle tab keeps no room for a state it isn't showing, so its × follows its name with nothing
idle before it, and the tab grows by the state's width when its agent starts (decided after
0.11.0's review, which measured 72 to 96 px of nothing before idle tabs' ×; the width had
been kept so a tab didn't move as its agent started and stopped). Behind the tab in front, in
a row too short for every tab
whole, a program's tab keeps its mark and what it's doing at either end, and its name between
them only where every tab behind has room for a letter and the "…"; with less, the mark and
the state stand side by side in its middle, on every tab behind alike: letters didn't tell
Comments from Codex, and the marks do. Where there are pictures the Agents tab has the mark its icon at the
top right has, and is cut short as an agent's tab is, its mark kept.
The page's menu folds formatting and inserting into Format › and Insert ›, and the agents
panel gives each agent one row under two lines of how-to. At every width a page keeps a
cell for a comment's ◆ and one for a change's bar, and a long page scrolls past its end
until its last row is a third of the way down. In code the ◆ of a commented line is in the
gutter's first column, which is against the sheet's edge: where there are pictures it's painted
half a column in, 8 px from the edge, and at the code's size, 16 px as a page's is, where the
line's number leaves the next column free; beside a number that starts there it's seven tenths
of that (it was seven tenths everywhere, 11 px with 34 px free after it). On cells it's that
column's letter.
The side pane or one editor can be maximized, from the pane's own tab row or a double-click on
a tab. The side pane takes the keyboard as it takes the window, as a click on it would, when it
shows a session or the Preview. The
icons at the top right never move for it. Whatever asks for a
hidden part, such as one of those icons or opening a file, restores first, so nothing opens
out of sight. A pane hidden while it has the window gives the window to the file, and takes
it again when it's shown, by its button in the corner or any other way: it comes back as it
was, as VS Code's maximized panel does. The pane opens at its share of the window, 46%, the
file keeping 44 columns and the pane 40; dragged, the line between them goes on until either
side has 24, which is what makes a drag worth anything in a window of 90 columns. The side pane stands under the file when it's dragged there, as well as when the
window is too narrow. Its width or height, whether it was dragged under the file, and whether it
had the window are kept in `state.json` with the tabs, so the next run puts it back as it was; it
takes the window again only once something is back in it. The session in front is kept too, by
its host's socket, so a run that ended on an agent's chat starts on it again. An editor's maximize lasts while
running. Panes split side by side only, so a tab
dropped on a file splits left or right, and there is no split below.

### Code

Code is laid out as rows rather than lines (`rows.rs`): a line each, unless word wrap breaks a
long line at the pane's width or a fold hides some. Scrolling, clicks, the caret and Up and Down
all go by row, so a wrapped line reads as several and a fold as one. Wrapped rows after a line's
first have no number and keep its indent; wrapping is off by default, Alt+Z changes one file, and
Settings changes the default.

Folds come from the language server's `foldingRange` when it has one, and otherwise from the
indent, as VS Code's own fallback does; a line of closing brackets folds in with its block and
shows after the fold's ⋯ chip, so a folded function reads `fn a() { ⋯ }`. The gutter keeps two
cells for chevrons, shown while the pointer is over it, and a folded line always shows its ›. A
fold follows its header line through edits, found again by its text, and opens when a caret lands
inside, whether from a search, a jump or a click. The headers of the scopes scrolled into stay
pinned over the top rows, at most five, with a soft shadow under them on pixels and a line on
cells; in the dark theme a hairline as well, where the shadow starts, since on graphite's sheet
a shadow is 1.05 to 1 and the pinned lines ran into the code. A scope's header is pinned while
its body has a line below the header's own row; one whose last line would be under its own
header is pinned all the same when, left out, that line would be the first under the others, so
no line shows without the header of the scope it's in (a `match`'s last arm stood under the
`while` with no `match`). A line of only closing brackets says nothing of a scope, so a scope
with only its `}` left in view isn't pinned. A line that's revealed, by Go to Line, a search's
result, a problem, a symbol or a definition, stands under the pinned headers, never behind them,
as in VS Code. Brackets take one of three colours by depth, gold, orchid and sky as in VS Code, toned for
each theme, with a stray in the error colour and the caret's pair tinted. Pages are none of this:
they lay themselves out.

### Markdown

Markdown follows Xcode 27's rendered editor, as read from its `RenderedEditor` framework:
one editable surface, not a preview beside the source. Block markers stay hidden even on
the caret's line. Bullets and checkboxes are drawn, tables have real cells, and fences
become a language label. Enter, Backspace and Tab act on structure. The source remains the
document, and each visible character keeps its source position. Xcode's handling of inline
markers wasn't confirmed: Cellix hides them too.

On the pixel tiers the page is proportional text at a readable width, with large headings
spanning two rows, tables with borders and padding, and rounded code blocks with a language
label and syntax colours. Xcode's formatting toolbar isn't built.

## Modes

A single input layer maps keys to editor commands.

- **Simple (default):** VS Code-style keys, mouse for everything, no modes to learn.
- **Vim:** built-in emulation, because Vim has no embedding interface. A subset: motions,
  counts, `d`, `c`, `y`, `p`, `x`, `u`, Visual and Insert. No `:` commands or registers.
- **Neovim:** drives `nvim --embed`, using the user's config. Requires `nvim` on the host.

With `vim` or `neovim`, the boxes written in, the comment and reply box, Ask's field and the
commit message, follow Vim too, through the built-in engine whichever of the two is picked:
Neovim has no box to embed in, so the user's own Neovim config doesn't reach them
(`App::box_vim_key`). A box opens in Insert; Esc goes to Normal, where motions, counts, operators
with motions and text objects, Visual, `u` and Ctrl+R, `y` and `p` with the clipboard work on it as
on the file. In Normal with nothing pending, Esc and Enter are the box's own, as in every keymap:
Esc closes it, Enter sends it. The box says its mode at its right, in its quiet words, as the
status bar says the file's. The caret stays a bar, as the file's Vim mode draws it. One-line boxes
and lists (the palette, Find, prompts, Go to Line, Settings' filter, Search's boxes, New
Worktree, the emoji picker) stay natural in every keymap, and Esc closes them at its first press,
as VSCodeVim leaves VS Code's input boxes alone. Settings' keymap row says so.

Order: Simple, then Vim, then Neovim.

## Scope: VS Code's built-in features

The baseline is VS Code without extensions, for what a developer does every day.

Already at that level: explorer with create, rename, delete and drag; tabs, splits, command
palette and quick open; project search and replace with expressions; multiple cursors and
tree-sitter highlighting; LSP definition, references, rename, completion, diagnostics, quick
fix and formatting; Source Control with staging and commit, blame, and side-by-side diffs;
the integrated terminal; notebooks; the minimap; settings and themes; Vim and Neovim modes.

The gaps found in daily use as a standalone editor (2026-11-07), all closed in 0.6.0:

| Area | What was missing |
|---|---|
| Code intelligence | Go to Symbol in the file and in the workspace, signature help, inlay hints |
| Editor view | Code folding, sticky scroll, a word-wrap toggle, bracket pair colours |
| Git | Merge conflict resolution, file history and a commit list with a light graph, stash, compare with any commit |
| Workflow | Run tasks (scripts, cargo, make, tasks.json) with their errors in Problems, format on save, auto save, `.editorconfig`, a keyboard shortcuts page, Open Recent |

Out of scope, said plainly: VS Code's extensions and marketplace, and settings sync (the
settings file lives in dotfiles). What sets Cellix apart is kept measured, so features can't
wear it away quietly: start-up time and memory against VS Code, and running over SSH with
nothing to install.

### Languages

No editor supports literally every language. Support is data-driven: a registry maps
languages to a tree-sitter grammar, with a TextMate grammar as fallback, and to a
language server. Adding one needs no code change. Over SSH, servers run on the
remote host and must be installed there.

Installing a server (Settings' Editor page, or the card that offers a language's support) runs
the first of its ways this machine can run, never asking for a password: an install has no
terminal, so sudo couldn't ask (`install.rs`). Homebrew where it's on the PATH, as on a Mac or
with Linuxbrew. npm installs globally, into npm's own folder where the person can write it and
with `--prefix ~/.local` where it's the system's, as with Amazon Linux's nodejs; npm's folder is
worked out as npm does, from its variable, `~/.npmrc`, or the folder over node's, without
running npm. Where the project builds its server for each system (marksman, rust-analyzer,
taplo, texlab, zls, terraform-ls), that build is fetched for this system and processor into
`~/.local/bin`, with curl or wget. `rustup`, `go install`, `dotnet tool` and `gem install
--user-install` run as they are. Where only a system package can give it (clangd on Linux, as
`sudo dnf install clang-tools-extra`) or only Homebrew (jdtls), there's no Install that could
only fail: the row says what's needed and has Copy Command, as the agents' cards do (one name
for that button everywhere). A row keeps the server's program in its middle column, and before
its button says how it installs: at rest a word or two, muted, the tool ("npm", "brew", "go"),
where a build comes from ("GitHub") or what it needs ("needs .NET SDK"); pointed at, the whole
of it in the same place and the same muted colour, what it needs still in small letters, cut in
its middle where the row is too narrow, never over the program's name; after a failure, why, in
the error's colour. The list keeps one right-hand column: a column for what a press does as wide
as the widest button any row has, Copy Command, on an Install row too, and every row's words
ending at one edge a column before it; the program stops 24 px before those words and is cut
with a "…" where it's longer. Tags ended at their own buttons, 85 px apart down the list,
"…-wrapper" ran to 10 px from "needs Homebrew" in the same grey, and pointed at, a row's words
went to a capital in the words' own white (the release review of 0.12.0, 42-settings-needs).
The row pointed at is washed whole, which leaves its button 8 px and more over and under, its
chip in the middle of the wash, where gone down onto the words' baseline it stood 10 px under the
wash's top and 8 over its foot. A button's label stands where it stands at rest, `BUTTON_ROOM`
in from the page's end, and pointing only adds its chip behind it (`Grid::row_chip`), reaching
that far past the label either side to the page's end: centred in a chip of whole columns,
Install moved 26 px left when its row was pointed at and Copy Command 31. Found, what's being
installed and the list's count end on that line too (`words_end`; the release review of 0.12.0,
42-settings-needs). The offer card says what its Install runs after what the server adds, a
build by its program's name, each a sentence with its stop; where it can't install here, what
it needs, then what Copy Command copies and what to do with it: "Needs Homebrew. Copies `brew
install jdtls`. Run it in a terminal, then open Main.java again." The card never said what went
on the clipboard or where to run it (108-install-card-needs). Its words are one paragraph, each
sentence running on from the one before, where a sentence to a line left the wrapped end of one,
"into ~/.local/bin.", a row under it as one more sentence (108-install-card-pane). A command is
set in the code font on a pill of the code's ground, as a page sets inline code, with 8 px of
the pill either side of it and 4 px after it to the next word, and wraps as one word: as words of the sentence, "Copies brew install jdtls." read the command as part of
it. Opening a file again that's open
looks once more for its language's server where it was missing, and starts it once it's there;
the bar keeps the next step after the card goes. A path or a command on the card never stands
alone on a line: the word before it goes with it ("into ~/.local/bin", "Runs `npm install -g
pyright`"). What a row says pointed at: "Runs npm
install -g --prefix ~/.local pyright", "Fetches marksman-linux-musl-x64 from GitHub into
~/.local/bin", "needs Homebrew, or sudo dnf install clang-tools-extra".

Cellix looks for a server, and starts it, in `~/.local/bin`, `~/.cargo/bin`, Go's bin
(`$GOBIN`, `$GOPATH/bin` or `~/go/bin`) and `~/.dotnet/tools` after the PATH, and in Homebrew's
LLVM, which it keeps off the PATH on a Mac, so an install that worked reads Found though the
PATH Cellix was started with lacks them; an install runs with them on its PATH too. An install
that fails says why in words to act on ("it isn't allowed to write where it installs. Run it
in a terminal", "the download isn't there", "no connection to the internet") on its own row
in the error's colour, until it's tried again, and on the card, which stays until it's
answered: the bar's message alone went with the next key.

### No debugger, for now

Agents debug through tests, logs and print statements, so it was dropped for the companion.
Standalone use will ask for it again; if it comes, it comes through the Debug Adapter
Protocol for two or three languages, after the gaps above.

## Agent integration

Each program in the side pane runs under a small host of its own: Cellix re-run as
`cellix session-host`, which owns the PTY and a Unix socket and keeps recent output, in memory
and in a log beside the socket. So Cellix can quit, or an SSH connection drop, and the next Cellix
in that folder finds it with what it printed. Quitting offers Keep Running. A dropped connection
keeps everything, since nobody can be asked. A host serves every Cellix that connects, as two
computers on one machine; all see its output, and the program takes the size of whichever typed
into it last, as tmux does. The program is told its socket (`CELLIX_HELD`), and an agent's hooks
leave its own session id beside it.

Nothing written to a program waits for it. Keys, pastes and a terminal's replies go into a
queue a thread of its own sends from, in order (`outbox.rs`), so a paste larger than the
terminal's buffer into a program that isn't reading no longer freezes every frame and key
until it reads; past 16 MB unread, a write is refused and the status bar says so. A host
does the same both ways. What a program prints goes to each viewer through a queue of its
own; a viewer 4 MB behind holds the program until it catches up, as a terminal holds a
program printing faster than it shows, but one that reads nothing for two seconds (a laptop
asleep with Cellix open) is let go, and the program and the other viewers carry on; it can
join again. What a viewer types goes to the program through one queue, so the host keeps
reading frames, a resize or the word to stop, while the program doesn't read.

A host that dies without finishing, as when the machine restarts, leaves its job, log and session
id; the next Cellix puts them aside and shows the tab as it was, with Resume (Claude Code's
`--resume`, Codex's `resume`, and the others' as below), Start Fresh or Start Again, and Close, and with others stopped too,
Resume All. It says "restarted" only when the machine booted after the log was last written.

### Where the keys are

A program in the side pane that asks to hear of focus (DECSET 1004) is told as the person's keys
come and go, as VS Code's terminal tells it of its window: `CSI I` as they come to it, `CSI O`
as they go. Claude Code asks as it starts, takes it that it has the keys until told otherwise,
and draws its prompt's block cursor only while it has them; Cellix never told it, so its cursor
showed with the keys in the file, which read as a place to type that wasn't one (the user, 22
November 2026). The keys are a session's while the pane is open with its tab in front and the
keyboard (`Dock::focused`), or a helper's while peek shows it, and nothing else has them: no
palette, menu, dialog or Find over everything, no comment, Ask question or commit message being
written, another workspace not in front, and Cellix's window not behind another. That's worked
out from where things stand each time the app settles, after an event or a poll
(`App::session_with_keys`), rather than at each of the sixty places the keyboard moves, and a
program is sent only a change from what it was last told. One that has just asked is told at
once only if it hasn't the keys; one joined again from its host is told either way, since an
earlier Cellix may have told it otherwise. The host follows whether its program asked and, when
the asking has gone from the 4 MB it keeps, asks again ahead of what it shows a viewer joining,
or the program would never hear again. A program that never asked is sent nothing. Peek has the
keys while it's open, as it had the pastes: keys went to the tab under its card while the pane
had the keyboard.

A terminal pane's own cursor, the blinking block a shell shows, follows the same: it shows only
in the session with the keys, so it's gone with the window behind another, as Claude Code's is.
VS Code draws an unfocused terminal's cursor as an outline; Cellix hides it, as it did whenever
the pane hadn't the keyboard, which also keeps the cell tier, with one cursor, the same. The
file's caret and its line's band go the other way: none while the pane or peek has the keys
(`App::pane_has_keyboard`), as an unfocused editor has none in VS Code, so only one place on
screen says it's where typing goes, and with the window behind another none does (the review of
this change found the file's caret standing alone there, with the keys in the agent). With the
keys in the file, its caret stays behind another window as before. The hint the first start
gives says so too: "Typing goes to Claude Code, on the right. Click the file or press Ctrl+` to
switch", where it said to click the pane, which had the keys already.

Where the keyboard starts: a workspace in front for the first time in this run, whether Cellix
started in its folder, it was opened, or it was restored and comes forward, has it in the agent
whose tab is in front of an open pane, so the cursor its prompt draws is where typing goes
(`App::place_keys`). Not over a dialog or Settings, nor where a file named at the command line
(`cellix file.rs`, and `--wait`) has it, nor for a shell's tab. A workspace already used keeps it
where the person left it. Inside Claude Code's pane Cellix restores no tabs, so nothing starts
there to take it.

### How the agent got there

Reviewing a result isn't only its diff: the steps, commands and reasoning that led to it can be
wrong too. An agent's hooks leave its transcript's path beside its socket with its session id, and
send it with every report, so Cellix knows each tab's transcript: Claude Code's
`~/.claude/projects/<project>/<session>.jsonl`, or Codex's rollout log, found by the session id when
a hook didn't say. Right-click an agent's tab, or run Open Agent Transcript, and it opens as a page
beside the code, made by Cellix into `.cellix/transcripts` (ignored by git) and read only: each turn
under a heading with how its context grew, the prompt in a callout, the replies as written, and
each tool call on one line with its input and a word on the result; one that failed says
`==failed==`, which a page shows as a chip in the failed colour's tint, the word in that colour
12 px from its ends (it was a highlighter's amber 2 px past the word). A command's output, and any
that failed, follows in a code block, cut to its last 12 lines; a file read whole isn't repeated.
What the transcript holds is data: tool output stays in code blocks, and nothing in it is run. A
selection on it gets Comment and Ask Agent as on any page, and the comment carries the step it's
on (`turn 1, step 3 (Bash cargo test orders)`), which is what the agent is told rather than a line
of Cellix's copy. The page is rewritten while it's open, as the transcript grows.

The same lines say how many tokens each reply took in, so each agent's tab shows how full its
context is, as a percent after its name: grey, orange from 80%, red from 95%, with the numbers and
the session's totals on hover, and the workspace's tile shows its fullest agent's. Context is the
last reply's input with cache, cleared at a compaction until the next reply; the window is what
Codex logs, and for Claude the model's: 1M for every model since Opus and Sonnet 4.6 and for ids
not known, 200k for 4.5 and older unless named `[1m]` or used past it. Transcripts are followed a few times a second, away
from the screen, reading only what was added.
### Agent context

Much of an agent's quality is won in what it reads and is told, so Agent Context, a view
on the rail, shows what an agent works from in this project, the way VS Code's views list
things: its instruction files in the order it reads them, with what each imports beneath it
and the ones it loads only on reaching a folder; its skills, subagents, commands, hooks and
MCP servers with their scope; and, for Claude Code, what its last session read and how full
its context got, with roughly what filled it. Every row opens its file, at its line, as a
page to edit, and Cellix writes only what the user types.

What Claude Code's transcript says it loaded wins over what the files suggest: it records
the instruction files in order, the skills, subagents and MCP instructions it was told of,
and the system prompt and tools. Without a session, each agent's own rules are applied to
the files (Claude Code's `CLAUDE.md` levels, rules and auto memory; Codex's `AGENTS.md` from
the repository's top down, to 32 KiB; Gemini CLI's `GEMINI.md`, or the names its settings
give). The picker moves between Claude Code, Codex and Gemini CLI, each named with its own
mark, as its tab has it (a blue dot stood before each alike). All of it is read as data.

Checks are gentle and never block: the same command told to run in one file and never to
run in another, an import that doesn't resolve, a file cut where the agent stops reading
(auto memory after 200 lines), a large file loaded every session, a skill or subagent with
no description, a hook that runs a missing script, and instructions kept in a file this
agent doesn't read. Whether two instructions disagree in meaning is for the agent to judge,
not for Cellix to guess.

### Tasks: the unit of agent work

Cellix shows agent work by task, not by file. A task is one agent session's turns, by the
agent's session id, so Resume keeps it, split where the session sends a new plan after one
was approved: the prompt that led to the new plan begins the next task. The reviewer can name a
task and start a new one at any turn; tasks are never merged. Decided with the user on
2026-11-07, over "a plan through its approval" (which leaves out agents used without plan mode)
and "a branch or worktree" (one endless task in a single checkout).

The agent's hooks append what happened to `.cellix/tasks.jsonl` in the project: each prompt
with when, which agent and session, the checkpoint taken as it began and the newest comment
then; each plan version and its answer; each test run. The reviewer's splits and names go in
the same log. Tasks are worked out from the log whenever it's read, so no two writers need to
agree on them, and a log line that can't be read is skipped. A turn runs from its checkpoint to
the next one taken in the project, or to the tree now, and the comments made while it was the
latest are its comments. Nothing is logged outside a repository or a folder Cellix serves.

The Task page (Show Agent Task) is a tab in the strip, after the files', as Settings and a
transcript are: its mark, "Task ·" and the task's name, cut short when long. It stays there
while a file is in front, a click brings it back as it was left, and its × or Close Tab closes
it and not the file behind. It had stood where the file would with no tab, so nothing said how
to leave it or get back (the canvas's board B, picked for the user on 2026-11-11). Under the
strip: the task's name with
a list of the others, then the timeline of plan versions and turns beside the chosen step. A
turn shows its prompt, what it changed, its tests and its comments, with Review Turn (Review
limited to that turn, read-only, since the tree has moved on) and Undo Turn (back to its
checkpoint, itself undoable). A waiting plan has Approve, Send Back and Compare. Open Transcript
opens the session's transcript as the transcript page does, and each turn shows how full the
agent's context grew over it, from that transcript. The line between the two columns is a
hairline, as the header's rule is, and stops as far short of the page's foot as it starts under
that rule. A plan's words, and what was said about it, are wrapped as they're drawn, measured,
and a heading in it has no empty line under it, so it stands nearer what it heads. Narrow, the
step's place stands in the middle between the two buttons that step, whose chevrons are icons.

### To-dos: what is left

A Task says what an agent did; a to-do says what is still to do, and the person and the agents
share one list. To-dos live in `.cellix/todos.jsonl`, one event a line — add, set, group,
edit, active, remove, clear, and an agent's whole list as a replace — and the list is worked out
from it whenever it's read, as the task log is; a line that can't be read costs only itself.
Every event carries its source (`you`, or an agent's session), so a replace touches only that
session's items.

One level of sub-tasks: a parent's state is its children's — done when all are, in progress when
any is, to do when none is — and its own state is ignored while it has children. A press on its
box groups them; a done child under an unfinished parent stays visible. Done top-level groups
fold under "Show N done", and Clear done takes only those.

Claude Code's `TodoWrite` is mirrored by the PostToolUse hook as a replace for its session. New
items match the old by text and then by list position, so a matched item keeps its id, its time
and any children the person nested under it, and an unchanged list writes nothing. Each event
carries Cellix's ids, so every reader of the log agrees on them.

### Agents starting agents

An agent can start others in the side pane, give them work, wait for them and read what they
said, with `cellix agent run|send|read|wait|ls|stop` (herdr's idea, built on what Cellix already
has). Each call is one line over the workspace socket of the Cellix that has the folder open,
`agent <json>`, and one line of JSON back; the app keeps a call that takes a while, a start or a
wait, and answers it on a later tick, each by a deadline of its own, so a caller always hears.
With no Cellix there the command says so and how to open one.

Every session Cellix starts for an agent or a command has a name, unique in the workspace,
`[a-z][a-z0-9_-]{0,31}`, the kind and a number unless `--name` gives one (`claude-1`), and is
told it in `CELLIX_NAME` beside `CELLIX_HELD`. Shells and the browser have none: they're nobody's
to message. The name goes into the host's job, so the next Cellix knows it again. A hook's report
carries the name from its environment, and Cellix takes it for that session only from the
session's own program (its process), not from another agent that program ran. So what each
agent is doing, working, asking the person, done, stopped on an error or exited, comes from its
own hooks; a program with none is working until it ends.

- `run` starts the agent in a tab of its own without taking the keyboard or the front, waits
  until it's ready, and types the task in. An agent Cellix knows is ready when its start hook
  says so, for at most 60 s; anything else when its output has been still for 800 ms. Quiet
  output alone never counts for an agent, since a question such as whether to trust the folder
  is quiet too and a typed task would answer it. It prints `name`, `kind`, `started` and whether
  the task went in.
- The first time an agent (a session with a name, or a caller with no terminal or an agent's
  mark) starts another, the person is asked in a card: the asking agent, what it would start
  and as whom, the task, and that it runs as they do and costs its own tokens; Deny, Always
  Allow Here and Allow. The card stands in the side pane a row under its tabs, a column in from
  each side, however wide the pane, or in the window's corner with no pane. Its buttons stand at
  the end of its foot with Allow the accent and Deny and Always Allow Here quiet buttons alike,
  as every button that isn't the answer is: the reference board had Deny as words, which beside
  two chips read as no button (the review of 0.11.0's tour). The board's 18 px inset is one
  column here, from the sheet's edge on both sides, the rule for every card. Its buttons end as
  far in from its right edge as its words start from its left, its panel's half column painted
  past their cells (they ended 33 px in where the words start 24 in). Always is kept in the
  folder's `state.json`. The asking agent is the sentence's subject by what it is, "Claude Code
  wants to start Codex", with its tab's name after it in the card's quiet colour, cut short
  with "…" or left off where there's no room ("· Review the queue"): tabs are named by their
  work, and as the subject such a name read as an order. The inbox and the bar say it the same
  way. `run` waits 90 s for an answer and says what happened. While the card
  waits, the asking agent waits on the person, and its tab says so with the needs-you dot, as
  its row in the Agents list, its workspace's chip and the bell do: it turned, and the chip
  said "working" (0.11.0's release review, 170).
- `send` pastes, bracketed where the program asked for that, and enters 200 ms later. It waits
  while the person types in that agent: 3 s after their last key, and with a prompt they've
  begun, until they send or clear it, at most 60 s after their last key. It refuses, typing
  nothing (exit 4), while the agent is asking the person something, since the text would answer
  it; `--force` types anyway. From one agent to another the text arrives as
  `<cellix from="NAME">…</cellix>`, which can't be closed early by what's inside it; from a person
  or a shell it's as typed.
- `read` gives the agent's last reply from its transcript, or the end of its screen; an agent
  whose tab has closed is still read from what it last showed.
- `wait` returns when the agent stops working or when a message is typed into the caller's own
  session, at once if it isn't working, and checks in after 180 s with exit 124: still working,
  wait again. After `send --wait`, the agent's last word from before the message doesn't count.

An agent started by another is that one's child; the parent is the session the caller runs in
(an agent's, not a shell's: a person in a shell starts agents of their own). `wait --children
all` lasts until none of the caller's children is working, and `--children any` until one has
stopped since the caller last heard of it. When a child needs the person, stops on an error or
ends, Cellix types a note into its parent, `<cellix>review needs you (permission). See: cellix
agent read review</cellix>`, once for each time it happens: not about a child the parent is
waiting on (the wait tells it), not while the parent itself is asking the person something, not
into a command, and with the same hold as a message while the person types there. Reading the
child counts as hearing of it. Children end with their parent, when it ends or its tab is
closed, unless started with `--outlive-parent`, which then go on as agents of their own.

In the Agents list a helper stands under the agent that started it, a step in, with a line bent
round from its agent's mark, by the name it was given and the quiet word "helper" after it,
whatever it's doing (the reference board's panel 4). A section counts the agents in it, not
their helpers: "WORKING 1" over an agent at work and the finished helper under it, where it
said "WORKING 2". A helper that asks the person brings its agent, and the agent's other
helpers, up to "NEEDS YOU", so what asks never stands under a quieter heading. For a while a
helper doing something else than its agent stood in a section of its own as "from" its agent,
and read as one more agent (0.11.0's release review, 172 to 174). Its mark at its row's end is
its own, and is what peek's header says of it, both read from one place: the spinner while it
works, a dot once it has finished a turn or is ready for one, the chip while it asks (its row
said "IDLE" by its hook's word while peek said "● done"). The line
bent round to it is a fifth of the way from the sheet to the words (#45474a on the dark sheet;
the sheet's hairline, 1.3 to 1, could hardly be seen), and starts 8 px under its agent's mark.
Before its hooks have said anything it's there all the same, doing what Cellix knows of it.

A helper has no tab of its own, so the pane's row stays the person's: a press on its row opens
it in peek, its live terminal on a card over the side pane, a cell in from it (a column at the
sides, a row over and under), with what's behind dimmed (panel 5). The card's header, its first
row, has its mark, its name in bold, what it's doing as a mark and a word in that state's
colour (the spinner and "working" in the blue, a green dot and "done" once it has finished a
turn or is ready for one, the attention colour's dot and "asking", the red's and "error"; its
row in the Agents list has the same mark, where a ready helper had none under a heading that
counted it, and "done" was grey words beside a working one's spinner, 0.11.0's recheck),
"· from" its agent's tab, quieter, Keep as Tab and ×; a line under it, then the terminal, sized
to the card, a column inside its panel. The header's parts are laid out by what each measures,
from their ink: 16 px between the name and what it's doing, a mark 8 px from its word, and
8 px either side of the "·" before who started it (it stood 20 px from the word and 8 from
"from"; by cells, "from Claude…" was cut with 82 px to spare, and the gaps ran 10 to 21 px). Who
started it is whole or not there: "· from Cl…" said nothing in a narrow pane. Keep as Tab is a quiet button, as the
board has it, at its height on the terminal's line as a button on a pointed row is
(`Grid::row_button`), which keeps 9 px from the card's edge and from the line under the header on
a row with room. Its × is as far from the card's right edge, to its ink, as the helper's mark
is from its left, about 24 px: four columns in, it stood 46 px from the edge (0.11.0's release
check, 173-peek). The
keyboard and pastes go to it while it's open; a press outside it or its × closes it, and the
press does nothing else. Keep as Tab gives it a tab of its own, in front. A helper that outlives
its agent gets one too.

An agent's name in any terminal's text, as in what one agent said to another, is a way to it:
Ctrl+click on the name of a live agent opens a helper in peek, over the tab where the name was,
and brings one with a tab of its own to the front, as their rows in the Agents list do. Other
words are as they were, a `file:line` opening its file, and a web address opens as a link does,
before any name in it is looked for. It's Ctrl on macOS too, not Cmd: a
terminal keeps Cmd with a press to itself and tells the program under it nothing of it.

`cellix agent --skill` prints a short guide for agents, and the same guide ships as a skill with
the Claude Code plugin, the Codex plugin and the Gemini CLI extension, which all read
`skills/<name>/SKILL.md`; Gemini's GEMINI.md also points to it.

### Starting things: the + and roles

The + after the side pane's tabs opens a menu dropping from it, as every menu here drops from
what opened it (decided 17 November 2026, after herdr: it showed the Agents list in the pane,
which took the pane over to start one thing). Terminal comes first, then every agent Cellix
starts in the order it knows them, each with its mark: one installed is picked by its digit
while the menu is open, 1 to 9 down the list, and Terminal by T; one not installed is dimmed
with "not installed" at its end, and does nothing, since installing is the person's to choose
(Settings' Integrations says how). Then ROLES, and then New Worktree…. The list in the pane
stays, for what's running.

A role is a small prompt file that starts an agent on one job: Markdown, its front matter
saying which agent runs it and what it's for in a few words, its body the agent's first
prompt.

```markdown
---
agent: codex            # claude, codex, gemini, or the name Cellix gives one; left out, the first installed
description: reads the changes, leaves comments
name: Reviewer          # left out, the file's name: code-reviewer.md is "Code reviewer"
---
Review the changes on this branch…
```

They're read when the menu opens, from `.cellix/roles/*.md` in the project, then
`~/.config/cellix/roles/*.md`, then Cellix's own, which ship in the binary (`crates/cellix/roles`,
one line in `roles::BUILT_IN` each); a name found sooner hides the same name later, so a project
can replace a built-in one. A role's row has its agent's mark, its name and its description,
quieter; one whose agent isn't installed is dimmed. Cellix ships Reviewer, which reads the
changes and leaves what it finds as suggested comments, and Tester, which runs the project's
tests the way its files say to and reports what failed; neither changes anything.

Picked, a role starts its agent in the pane, as the agent's own row would, with the role's body
as its first prompt. That's given the way each agent takes a first prompt when it starts
(`claude -- <prompt>`, `codex -- <prompt>`, `gemini --prompt-interactive=<prompt>`;
`agents::first_prompt`), not typed into its screen: the agent takes it once it's ready, after
whatever it asks first, and a new folder's trust question is the first thing Claude Code asks
in a worktree. A prompt typed when the screen went quiet would have answered that question
with its Enter. An agent Cellix doesn't know how to give a prompt to gets it on the clipboard
instead, with a word saying so.

### Review with an agent

Review's list ends, a row under its files, with its buttons: Commit with how many files it
takes, "Commit · 1 staged", where any are staged, filled, and Review with…, a quiet button, each
as wide as its words. They stand side by side where both fit, and one under the other in a list
too narrow for both, as Review's own 24 columns is (Commit filled the list's width and said
"Commit" alone, under the last file with no row of air). The palette has Agent: Review Changes
with… for where there's no list. Both drop a menu of the agents installed here, each with its
own mark, and after a rule, Set up more agents…, which opens Settings at Integrations: a list
shows what a press on it acts on, and four of its six rows were agents not installed, dimmed,
doing nothing (0.11.0's review of `next`, 175). From the button it stands 8 px under
it, hung as a menu under a line is (`Menu::hang`), where it stood 3 px under the button's foot. The one picked starts in the pane as the Reviewer role does,
called reviewer while that name is free, its first prompt the role's with its first line saying
which changes Review compares and the git command that shows them: since the last commit (`git
diff HEAD`), since a turn's checkpoint, or one turn's own (`git diff <before> <after>`). A
project's own Reviewer role is used in place of Cellix's; one whose first line doesn't name the
changes is told them before the rest.

The role tells it to change nothing and to leave each finding with `cellix suggest`, which makes
a comment suggested by that agent. Review shows each under its line in the cards, after the
person's own: "Suggested by Codex · Clarity" in the accent, as the Comments list says it, its
words cut with an ellipsis, and Keep and Dismiss at the row's end: quiet buttons at their height on the
terminal's line (`Grid::row_button`), which on a one-row note keep 9 px from its edges, and the same two
quiet buttons on the suggestion's card in the Comments list, where Keep had been filled and
Dismiss words, and under the line both had been words in the accent.
Kept, it's the person's comment, and goes to the agent as theirs do, one on code with the review
and one on a document as a thread (a kept one on a document was left as told, so it never went); dismissed,
it's resolved and gone. The Comments list's Keep and Dismiss do the same.

### Workspaces, in place of tmux

Cellix keeps the open tabs, cursors and unsaved edits of each folder in `state.json` beside its
sessions, written on the tick when they change and on exit, and opens them again on start, so a
dropped connection or a restart loses no typing; edits made before the file changed on disk come
back marked as a conflict. Answering Don't Save on Quit keeps the tabs but not the edits.

One Cellix holds several folders, each a whole app with its own socket, IDE port and sessions.
A bar on the bottom row, shown even with one folder so opening another is always in reach, is
the terminal's own colour like the status bar, so the window padding shows no seam. It has a tile
per folder that says what its agent is doing, a labelled New Workspace button to open
a folder, Resume for everything that stopped, and the machine's name. Each folder's sessions folder
names its project, so a new Cellix opens every folder on the machine with something running in
it, or marked open (below), that no other Cellix has open, and Cellix started at home with nothing
named offers them first. A folder where something only stopped (the machine restarted under it) is
offered to resume, in the folder picker and `cellix ls`, but not opened by itself: until 0.15.1 it
was, so a workspace closed from its tile came back at every start. A session stopped on purpose,
as Quit or closing a tile stops it, leaves nothing: its host lets go once the program ends, with
nobody left to tell, and ends one that ignores the hang-up three seconds later. It used to wait
for a viewer to hear that the program had exited, and the next Cellix took it for one running. That first screen can be left as any other: Quit beside Open, the button at the end of its foot, or the key
that quits, and every folder stays as it is for next time, with nothing asked. Every folder is marked `open` there too from when it's opened as a workspace, so it comes
back with nothing running in it, as an editor brings back its windows, however the Cellix that
had it ended; closing its tile lets it go. Until 0.10.1 the mark was made as Cellix quit or
hung up, and a Cellix that never got that far (killed, or cut off with its connection before
it had finished leaving) left the next one only what had something running: the first screen
offered one workspace of several. Closing a tile asks as Quit does; Quit asks each folder in turn. The bar is drawn on the
same grid as everything else, so it works on the cell tier too.

A worktree gives an agent a checkout of its own, so two can work at once without writing over
each other. Cellix listed them (the Agents list's WORKTREES) and from 0.11.0 makes them: New
Worktree… on the + and in the palette (`Git: New Worktree…`) asks for a branch's name, says
where the folder goes as it's typed, and offers the installed agents and Nothing to start in it,
the first agent picked. Create runs `git worktree add` from the HEAD in view: a new branch, or the
branch where one by that name is already there, checked out. The folder opens as a workspace of its
own, beside this one in the bar, with the agent picked started in its side pane. What git
refuses, a name that can't be a branch's or a folder that's there, is said under the box, which
stays.

Where it goes, decided 17 November 2026: `<project>.worktrees/<branch>`, a folder beside the
project's own checkout, `queue.worktrees/fix-pop-order` beside `queue`, a `/` in the branch's name
a `-` in the folder's. herdr keeps them under `~/.herdr/worktrees/<repo>/<branch>`, out of sight.
Cellix puts them where the person keeps the project: a worktree is work to review and open
again by its path, which is plainer beside the project than in a tool's folder in the home
folder (Cellix has none there; its own files are a cache it may clear). Beside it rather than
inside, so the project's explorer, search and `git status` never see a second copy of the tree.
And one folder holds them all, which is one thing to look at or remove. From a worktree, a new
one goes beside the project's own checkout, not beside that worktree.

A worktree's row in the Agents list has Remove Worktree… on a right click (and Review Changes, as
a left click does), greyed for the project's own checkout and for the folder open here. It asks
first; a checkout with changes not committed is refused unless asked a second time, and then
removed with `--force`. The branch is never deleted, whatever is answered: its commits are still
there to merge or to check out again.

What waits for the person, in any workspace, is kept as it happens: an agent asking leave to
run something, a plan to review, a finished turn, an agent's answer to a comment, a failed test
run. Each workspace keeps its own list and drops what's dealt with (answered, approved, resolved,
a passing run, or opened when it only wanted seeing). The Inbox in the side pane, behind a bell at
the right end of the status bar as in VS Code, lists them all newest first, and a click goes to the workspace and opens the
thing. While the terminal says Cellix isn't in front (focus events), new ones become the
terminal's own notification, so they reach the person over SSH with nothing installed: OSC 777
where the terminal is known to read it (Ghostty, WezTerm, foot), OSC 99 for Kitty, OSC 9
elsewhere, wrapped for tmux's passthrough inside tmux. One goes at most every 10 seconds, a
burst becoming one, and Settings turns them off. A terminal that sends no focus events never
gets one; nor does tmux without `focus-events` and `allow-passthrough`.

On Linux over SSH, where logind ends a user's programs at logout and lingering is off, a card
says so the first time a session starts and offers `loginctl enable-linger`.
Claude Code is told the IDE port when it starts, so Cellix keeps its port and token per folder, and
the next Cellix takes the same ones for an agent that outlived the last.

Find follows the keyboard. In a terminal's tab it looks through all the terminal holds, the
scrollback too, with the file's own Find bar standing on the terminal's first row: what it finds is
marked, Enter and Shift+Enter step through it from the newest, the view scrolls to the one it's on,
and what the terminal prints meanwhile is looked through as it comes. There's nothing to replace
there, so that row can't be opened. An agent that keeps a screen of its own, as Claude Code does,
leaves the terminal no scrollback: Find in its tab opens its transcript, which holds the whole chat,
and looks there. As in VS Code, this takes Ctrl+F from the program in the terminal.

Find follows where the last press landed. On a file it's the file's. In a terminal's tab it
looks through the terminal's scrollback, and in the tab of an agent that keeps its own screen
through the agent's transcript. After a press on the side pane with a list in front of it,
Comments, the Inbox, Problems, Agents or a build's log, it looks through everything that list
holds, not only what's in view: the list is drawn once more from its top into a grid tall
enough for all of it (600 rows) and its rows are what's searched, so every list is covered by
the one way and a new one needs nothing. The matches in view are marked as they're drawn, in
cells or behind the painted letters, and which of the list's matches they are is told from
their order, since each list scrolls in its own rows. Review and the Task page are looked
through the same way when one of them is what's showing (`find::Looked`): Review's changes,
and the Task page's timeline and the step beside it, which are two parts that each scroll on
their own. Where a part scrolls a row at a time the match's own row says how far to scroll;
only a part that scrolls by items is tried a position at a time. Ctrl+F in Review opened the
project's Search before; that's still Ctrl+Shift+F.

The side pane's terminal also reads the Kitty graphics protocol, so a program in it can draw
pictures. That is how `terminal-browser` shows a real browser inside Cellix, and how a plot from a
training run shows in a shell. Frames become bitmaps on the `Grid`'s paint list like any other
image, so a terminal without pictures falls back to the system browser.

### Pasting a picture over SSH

Cellix runs where the code is, so the clipboard it needs is the terminal's, on the person's own
machine. OSC 52 carries only text, and a dropped file arrives as a path on that machine. Kitty's
clipboard protocol carries any kind: with paste events on (mode 5522), a paste arrives as the
list of kinds the clipboard holds and a one-time password, and Cellix asks for the one it wants
with that password, which the terminal answers without asking the person
(<https://rockorager.dev/misc/bracketed-paste-mime/>). Text wins if there is any, so pasting
text is unchanged; otherwise a picture, PNG first. It's kept in the project under
`.cellix/pasted` (ignored by git) and pasted as its path, which an agent such as Claude Code
attaches as an image; on a page it's kept beside the document and put in, as a picture pasted
there always was. A TIFF, which a Mac screenshot can be, is kept as PNG.

The mode is asked for (DECRQM) with the questions at start and turned on only where the terminal
itself says it has it, with nothing between: Ghostty from 1.4 and kitty from 0.44.1. crossterm
knows nothing of the replies, so Cellix puts them back together from the keys crossterm makes of
them, as it does a Windows paste; with the Kitty keyboard protocol on in those terminals, a real
Esc or Alt+] can't be taken for one. `CELLIX_PASTE_EVENTS=0` leaves pastes as plain text.

What other setups can't do: tmux and Zellij don't pass paste events on, and a terminal that
doesn't answer that it has them (Ghostty 1.4 and kitty 0.44.1 do; others weren't checked) gets no
paste events, so there a paste is text only and a picture can't cross. There, copy the file to
the server (`scp`) or save it in the project and paste its path.

Cellix runs the real agent and never rebuilds its interface. The side pane is a terminal on a
PTY, using `alacritty_terminal` for the screen, so agents update themselves and nothing here
knows what they look like. A program's synchronized update (`?2026`) is held in the parser
until it ends, and changes nothing on screen meanwhile, so no frame is drawn for it; one its
program never ends, as when it's killed halfway, is ended 150 ms after it began, as terminals
do, with the loop waking then to end it (the parser only looks at the time when more comes). When a program there ends, as an agent does on `/exit`, its tab
closes and the status bar says so; an agent that ends within five seconds of starting didn't
start, so its tab stays with what it printed. Starting one in a host split left the file and the agent in two
programs, and real use showed that isn't side by side. It connects to the real tool in three ways, each proven
against Claude Code 2.1.309:

- **Hooks** reach the workspace socket through `cellix hook <kind>`. They checkpoint turns,
  report status and test runs, and hold plans. Only a permission or elicitation notification
  counts as the agent needing the person, any tool finishing says it's working again, and
  each event carries the agent's process, so one that is killed stops counting. Every hook exits cleanly, so a missing
  Cellix never gets in the agent's way.
- **The IDE server** is MCP over a local WebSocket, announced by a lock file in
  `~/.claude/ide`. Only a client with the token gets in, and any request with an Origin is
  refused, which keeps web pages off the port.
- **The mod** adds a status entry and slash commands.

Plans answer on `PermissionRequest`, not `PreToolUse`: a real run showed `PreToolUse` leaves
Claude Code's own dialog up, and the answer must carry `updatedInput` to count.

The plugin ships inside the binary and is written out when an agent starts. An upgrade
replaces the binary but not the files it wrote, so each Claude Code session's start hook
rewrites any file of the installed plugin that differs from the binary's, and Claude Code
picks the new mod up. The mod also tells `cellix pane` the protocol it speaks
(`CELLIX_MOD`); a pane that gets another refuses with a message the old mod shows, rather
than sending lines it would ignore.

### State from any program

A program with no hooks can still say how it is, through the Program Status Protocol herdr made
open: it prints `ESC ] 7501 ; key=value:key=value… ESC \` (or BEL) to its terminal. `state` is
`idle`, `working`, `done`, `blocked`, `error` or `clear`; `app` names it; `kind` says why it's
blocked (`permission`, `question`, `auth`); `progress` is 0 to 100; `title` and `msg` are base64;
`id` names a part of it, empty for the program itself. A script reports itself in one line:

```sh
printf '\033]7501;state=working:app=deploy\033\\'
printf '\033]7501;state=blocked:app=deploy:msg=%s\033\\' "$(printf 'Deploy to prod?' | base64)"
printf '\033]7501;state=done:app=deploy\033\\'
```

(`\033` rather than `\e`, which `dash`'s `printf` doesn't know.)

`alacritty_terminal` ignores an OSC it doesn't know, so the side pane's terminal lifts these out
of what comes from the session host before its parser sees them, as it does Kitty's pictures,
wherever the stream splits. What they say becomes the state Cellix keeps for agents, so it
shows in the Agents list, the workspace's tile, the inbox and the notifications as hooks' news
does: `blocked` is needing the person, with `msg` as what it asks (or a word from `kind`);
`working` says `msg` or `title` and how far along it is; `done` is a finished turn, `msg` its
detail; `error` is stopped, as a turn cut short is; `idle` is idle. `clear`, the program ending,
or its tab closing ends it, and a part's `clear` ends that part and its own. A part is a row of
its own, named after the program and the part. `app` naming Claude Code, Codex or Gemini CLI
(`claude`, `claude-code`, `codex`, `gemini`…) is called by Cellix's name for it; any other is a
plain program under the name it gave, and one that gave none goes by its tab's. A click on what
it asks goes to its tab, a shell's too. Where an agent's hooks report for the tab (they left its
session beside the host, or it's that agent's only tab, and the session is running), the hooks
win: the program's word is let go. A Cellix that joins a running host sees what the host kept,
so the last word is known again; a `done` heard that way isn't news.

### Inside Claude Code

`cellix pane` runs the whole app with no terminal, started by the mod for the session's
life. It writes each changed frame to a private file and prints one line, and the mod
swaps the pane's image to it. Claude Code shows a picture from a file only in kitty and
Ghostty on the same machine. Everywhere else, over SSH and through tmux, the pane goes by
value as a picture a row (`rows` lines). Each row is named by what it shows, a hash of its
cells and paint with tall shapes cut to the row's reach, and a PNG crosses only for a name the
mod doesn't hold; the mod draws a column of `Image`s keyed by name. Claude Code uploads a
picture to the terminal once per key and moves it with its row, so a click or a letter costs a
row or two and a scroll the lines that came into view (measured over SSH and tmux: a click
21 KB, a letter 28 KB, a scroll notch about 60 KB, where one picture of the pane each change
cost about 300 KB and blinked). Rows are palette PNGs where they fit in 256 colours, about a
third the size, and pages that are all new in quick succession go at half resolution, sharp
once the pane is still. Nothing is sent until the mod says the pane's size.

Claude Code decides whether it can show pictures inside tmux as it starts, from tmux's global
`allow-passthrough` and `focus-events`; a pane's own option, or one set later, isn't seen.
`cellix setup claude` adds both to the tmux config, and the session-start hook and the pane
set them on a running tmux server, so the next Claude Code started there shows pictures. The
mod checks once, by swapping a drawn row for itself, and where Claude Code draws the alt
instead, asks for the cell tier: `cells` lines drawn as a `Raster`. `CELLIX_TIER=cells` or
`pixels` overrides the choice, and `CELLIX_PANE_LOG=<file>` records what came in and went out. Input comes back through a surface module layered over the picture. It is
gathered per tick and sent as one batch per `cellix pane-send` over the workspace socket,
whatever piled up while the last send ran going in the next. Cellix keeps only the last of a
run of moves and takes everything waiting before it draws again. An idle pane sends nothing.

A process per batch costs a few milliseconds. If it shows, the next step is a long-lived
input channel.

Cellix runs one local socket per workspace, and each integration is a thin adapter.
When a second Cellix opens a folder another already serves, the one opened last takes the
socket. It's the one in front of the person. The other may be out of sight, left in tmux or
behind an SSH link that dropped. While the first kept the socket, an agent beside the second was
told it had opened its file, or that a comment the person could see didn't exist, by the
Cellix nobody was looking at. The older one notices it lost the socket, says so once, and
takes it back within a few seconds of the newer one closing. Each only ever removes its own
socket file, told apart by inode and creation time. A Cellix that is only editing a file
for another program (`--wait`) leaves the socket where it is. `cellix open` answers once the
file is open, and says where: in front, in a workspace that isn't, behind a proposed change,
or why it couldn't. The file comes out from behind Review, a maximized pane or the sidebar's
drawer, as it does for Claude Code's IDE `openFile`.
Comments reach an agent through `cellix comments [--take]` as JSON. With `--wait` they are
also appended to the file as a "Review comments" section, because `$VISUAL` only returns
the file. `cellix comments` lists the review's comments on code alone; `cellix comments
--waiting` lists everything an agent hasn't had, a document's comments and kept suggestions
too, and takes nothing, so the prompt hook still hands each over once. The Claude Code mod
counts "N comments waiting", and /cellix-send and Send start a turn, by `--waiting` (falling
back to `comments` with a Cellix from before it). It read `comments`, so a page's comments
weren't counted and /cellix-send said none were waiting.

- `$VISUAL`: `cellix --wait <file>` covers Ctrl+G in Claude Code and equivalents elsewhere.
- Review return path: comments are delivered as structured data over the socket and
  CLI, so any agent can consume them.

- Claude Code: a plugin with a mod for status and commands. The mod is a bridge, not
  the editor: docked panes stop at 45% width and there is no terminal element. Installing
  it sets `CLAUDE_CODE_PLUGIN_DIRS` and `CLAUDE_CODE_AUTO_CONNECT_IDE` in the `env` block of
  the user's settings, because a mod loads through that variable or `--plugin-dir`. This
  only happens when the user asks. It also sets `CELLIX_BIN`, where Cellix is, since Claude
  Code started from an IDE, a service or by its full path may have a PATH without it; the
  mod and hooks try `cellix` first and that second. A pane that can't be started says why.
- Codex: a plugin in a local marketplace, installed with Codex's own commands. It runs
  hooks for status, turns, comments and test runs, once the user trusts them with `/hooks`.
  A plan is no tool call there, so its Stop hook holds a `<proposed_plan>` block. Sending it
  back starts another turn, and approving leaves implementing it for the user to ask for.
- Gemini CLI: an extension with the same hooks, installed with its own command. Its folder
  trust question isn't covered by `--consent`, and the user's click is the answer to it.
  `BeforeTool` on `exit_plan_mode` holds the plan, and Gemini still asks for its own approval.
- OpenCode: a plugin of one file, `cellix.js` in its global plugins folder
  (`~/.config/opencode/plugins`), which it loads as it starts. Its events become the same hook
  calls (`cellix hook --agent OpenCode <kind>`), each a process the plugin doesn't wait for:
  `session.status` busy is working, `permission.asked` and `question.asked` need the person,
  `session.idle` is a finished turn, `session.error` a turn cut short, and a command's run
  comes from `tool.execute.after` with the command `tool.execute.before` saw. Resume is
  `opencode --session <id>`.
- Copilot CLI: a hooks file of Cellix's own in `~/.copilot/hooks` (`COPILOT_HOME` if set),
  its events named as Claude Code's are, which gets Claude Code's field names from it
  (`session_id`, `notification_type`). A `permission_prompt` or `elicitation_dialog`
  notification needs the person, `agent_idle` is waiting; `Stop` finishes a turn. Resume is
  `copilot --resume <id>`.
- Cursor's agent CLI (`cursor-agent`): its hooks go into `~/.cursor/hooks.json` beside the
  user's own, and come out again by what they run. It names the session `conversation_id`
  and gives its workspace's folders, which the hook takes as the session and the folder;
  its prompt hook answers `{"continue": true}`, since Cursor waits for that word. Cursor has
  no hook for asking the person anything, so Cellix can't say it needs you: only a program's
  own OSC 7501 report could, and Cursor sends none. `stop` says how the turn ended. Resume is
  `cursor-agent --resume <id>`, taking the conversation for the chat Cursor resumes.
- Kiro CLI (`kiro-cli`, started as `kiro-cli chat`, with no flag: from 3.0 its interactive chat
  is V3 unless the person chose V2, and `--v3` would overrule that choice and may be unknown to
  an older one): a hooks file of Cellix's own,
  `~/.kiro/hooks/cellix.json`, in the standalone shape its V3 chat reads in every workspace
  beside the workspace's and the person's (`{"version": "v1", "hooks": [{name, trigger,
  action: {type: command, command}, timeout}]}`). Its triggers call `cellix hook --agent Kiro
  <kind>`, falling back to `$CELLIX_BIN` as Claude Code's do: `SessionStart` is ready,
  `UserPromptSubmit` working, with the prompt from `USER_PROMPT` where its JSON has none,
  `PreToolUse` working, `PermissionRequest` (3.4 and later, run just before a call would ask
  the person) needs you, saying the command or the tool and the path it's on, `PostToolUse` a
  tool's end and a test run (`exit_status`), `Stop` a finished turn, `SessionEnd` ended. Its
  hooks print nothing: `SessionStart`'s and a `Stop`'s output would reach the agent, and a
  permission hook that says nothing leaves Kiro's own question to the person. Kiro CLI 2.x
  read hooks only from each agent's own file, which Cellix never edits: `cellix setup kiro`
  asks `kiro-cli --version` and says what an older one needs (3.0 for these hooks; 3.4 for
  needs you, where the trigger is left out of the file, since a trigger Kiro doesn't know may
  keep the file from loading), and how to turn V3 back on where V2 was chosen. Its sessions are
  in its own database, keyed by folder, with no transcript to follow, so it has no context
  meter. Resume is `kiro-cli chat --resume-id <id>`, the id its hooks give. `kiro-cli` typed at
  a Terminal's prompt is its tab too, as is its chat's own process, `kiro-cli-chat`. Settings
  › Integrations lists it, after Gemini CLI, only where `kiro-cli` is installed or its hooks
  are there to remove.
- None of the four can be handed review comments by a hook (Copilot's and Cursor's prompt
  hooks can't add to the prompt; OpenCode's plugin can't from where it's told; Kiro's docs
  don't say a CLI prompt hook's output reaches the agent, and a comment taken but never read
  would be lost), so their comments wait for Ask Agent in their tab, or an agent that can take
  them.
- They're offered under START only where they're installed (`opencode`, `copilot`,
  `cursor-agent`, `kiro-cli` on the PATH), and `cellix setup opencode|copilot|cursor|kiro` (or
  Install in Settings, for Kiro CLI) is the only thing that writes their config; `--remove`
  takes back exactly that. Written from each one's own docs, and proven with stand-ins only:
  none of the four has been run with Cellix.
- The icons at the top right, as in a desktop app, reach the comments, a terminal, Review, the
  browser and the agents. The pane's own control ends the row, three columns from the
  window's edge. It hides the pane and is drawn as a window with its right part filled, not
  an ×, so nothing in the corner reads as closing more than a tab. Quit isn't there: see
  "Quit, at the foot". A hidden pane leaves that control in
  its place, the window with its right part in outline, so something in view always brings
  the pane back as it was. The
  explorer lists gitignored files, as VS Code does, since agents work in them, but dims them,
  and what's dimmed is exactly what Quick Open and search leave out. The pane stands under the file when the window is too narrow to have
  them side by side. The Agents icon opens the side pane. With nothing running here, it says how
  it works, and lists what can be started and the worktrees.

The Agents list orders agents by how much they want the person: Needs you, Done, Working,
Idle, each a section headed in small capitals with how many are in it, and a section with
nobody in it left out. A row is the agent's mark, its name (its tab's, where it has one), how
long since what it's doing changed (`<1m`, `5m`, `2h`, `3d`, which the half-second tick keeps
up and nothing redraws faster for), and at its end what that is: the spinner, a chip, the
design's chip of 20/28 of a taller row with room under its words' tails (it was 24 px, the "y"
of "you" a pixel over its foot), saying "needs you" in the attention colour with what it's
asking on the row under it, its words' ink in its middle, raised the 2 px their tails reach
further down than their heads reach up (they had 11 px over them and 7 under), a green
dot, or nothing. Every row keeps room there as wide as that chip, an idle row's empty, so how
long ago stands in one column down the list, one kind of value to a column (it moved six
columns left on the row that needs you, and at an idle row's end it stood 139 px right of the
rest: the release review of 0.12.0, m2-agents-urgency). What stands at a row's end, the chip,
the spinner or the dot, ends as far in from the pane's right edge as the marks start from its
left (`measures::row_end`), so the band under the pointer has 15 px either side of what it
holds (it had 36 at its end). A line that parts the list, the one under Terminal in START, runs
from where the rows' marks start to where their ends end, 25 px in from the pane's edge either
side, as the Inbox's does: two columns in, they stopped 6 to 9 px short of the rows (the
release review of 0.12.0, 14, 51). The chip's words are the attention colour, deepened where
that reads under 4.5 to 1 on its wash (`theme::readable`), as in the light theme (4.28 to 1),
and so is a transcript's "failed" on its chip (4.4). A name is in the words' own colour, since a press on any row
does something: bold while its agent does something, at regular weight while it's idle or is a
helper under its agent; how long ago is what's quiet. An idle name in the quiet colour read as
the menus' "not installed", and a helper as bold as its agent read as its equal (0.11.0's
review of `next`, m2, 172). Under START,
what can be started is listed as the + menu lists it: Terminal first, a rule, then the agents,
one row each with its mark and what it runs at the row's end, where each had a + and "Starts
here, working on this project" under it. Done is an agent that finished a turn nobody has looked at: it stays Done
until its tab is in front. A row can stand under another, a step in with a line
bent round from its parent's mark, while the two are in one state (`agents::ordered`, by each
row's `parent`); in another, it's in its own section and says who started it.

Every row does something when pressed: it brings its tab forward, or opens peek for a helper
with no tab of its own; a part of what a program in a tab said (OSC 7501's `id`) brings that
program's tab forward. So the list shows only agents with a tab here and their helpers. An agent
running outside Cellix, in another terminal, isn't listed, whether its hooks report or a look at
the folder (`lsof`) found it, idle, working or asking alike (the user's choice, 19 November 2026,
after "There are idle agents session list in the agents tab but they are not actionable. Seeing
that list is not useful"). Nor is it counted where the list's agents are: the status bar's "An
agent needs you", the Agents icon's dot, the workspace's tile, and the bell and the
notifications it gives, which its news doesn't reach. It still hears about comments, as the agent
working on this project outside the pane (`Agents::outside`).

- Rebuilding an agent's interface: the side pane shows the real program, so its features arrive with its updates.

An agent's tab is named by the agent: by the title it gives its terminal, as Claude Code sets
it to what it's working on, with the glyph it puts first while busy taken off (one braille
letter, or one of `· ✢ ✳ ✶ ✻ ✽ ◐ ◓ ◑ ◒`, and the spaces after it; `dock::own_name`). A title
that's only the program's name ("Claude Code", "claude", or "Gemini" for Gemini CLI) names
nothing, and every tab read "Claude Code" (the user, 20 November 2026: "Can we have meaningful
chat titles similar to Claude Code when there are multiple chat sessions?"). Such a tab goes by
its conversation's title, as Claude Code keeps it in its transcript and its /resume lists it:
the latest `custom-title` line (one the person gave it), else the latest `ai-title`, else an
older version's `summary`, read with the context meter as the transcript grows; and until one is
written, by the first thing asked of it, the prompt hook's first line that says anything (a
command, as `/clear`, names nothing). Codex and Gemini CLI keep no title we could find in their
own files, so theirs go by the first prompt. Either is cut to 32 letters at a word's end, not
on a word that leads into the next ("Fix the login redirect…"), and the tab's tip has it whole,
over what its context is. The Agents list calls each agent what its tab says. Two agents' tabs
that would still read alike are told apart by the order they started: the later one's name has
" (2)" after it, the next " (3)", kept after the "…" where a narrow row cuts the name short
("Cla… (2)"; " · 2" after a "…" read as four dots); a shell's, the browser's and a renamed
tab's never do. Until
any of these, a tab goes by the program's name. A shell's title isn't used: it names the
machine and the folder. Rename… on the tab's menu, or Agent: Rename Pane Tab, names it over that; the name is
kept with the pane in the folder's `state.json`, by the session's host's socket, so the tab has
it again when the next run joins the session, and named nothing it takes its own name again.
What a tab's mark says comes from the agent's hooks, through the tab whose agent has that
session's id, or whose agent is the hooks' process (the `pid` their news carries), or else the
one running tab of that agent, and from a program's own OSC 7501 reports for its tab; hooks win
there as everywhere.

Naming doesn't depend on which `cellix` the hooks run. On a machine with an older `cellix`
first on the PATH, the hooks sent state, so spinners turned, but no prompt and maybe no
transcript, and Claude Code's tabs stayed "Claude Code" (the user, 20 November 2026, on Amazon
Linux: "Claude code titles are not changing automatically"). So Cellix also asks Claude Code:
it keeps a file for each process it runs, `sessions/<pid>.json` in its folder
(`CLAUDE_CONFIG_DIR`, or `~/.claude`), rewritten as things change, with the conversation's
`sessionId` and its `cwd`. The pid is the tab's own program for an agent's tab (a held one's
host writes it beside its socket) and, for a shell, the leader of the process group in front of
its terminal (`tcgetpgrp` on a PTY Cellix holds, and for a held shell the shell process's
`tpgid`, from `/proc/<pid>/stat` or `proc_pidinfo`, so a host of any version can be asked).
The transcript is `projects/<cwd with every character but letters and digits as ->/<id>.jsonl`,
or that name in another project's folder, as when the agent moved after starting there
(`transcript::Sessions`). The file and the transcript are read with the context meters, away
from the screen, every couple of seconds, the file again only when its time changes. The
transcript also gives the first prompt, which names the tab after its title and before the
prompt hook's word; a name the person gave in the file (`nameSource` other than `derived`,
which is the folder's) counts as a title. Each name belongs to a conversation: when the id a tab
holds changes, by its hooks or by Claude Code's file (/clear, /resume, a new session in the same
tab), its title and first prompt start over from the new conversation's transcript, and the
prompt hook's words name it only while it holds the conversation they were asked in. Claude
Code's file is taken only when it changes, so a read from before a /clear doesn't take the tab
back from the conversation its hooks have just reported.

An agent typed at a shell's prompt (`claude` in a Terminal tab) is named and marked as an
agent's tab is: its program is one of `AGENTS` by its command, or a process Claude Code says is
its own, whatever the system calls it (a real `claude` can run as `node`, or as its version's
file on macOS). Its tab says "Claude Code" with Claude Code's mark rather than `claude`, then
its title, its conversation's title or first prompt; only a title with an agent's busy glyph
first is its own there, not the shell's (`user@host: claude`). Its hooks' news finds the tab by
its process, its mark says what they say rather than turning as a command does, its transcript
gives its context meter, and the Agents list calls it what its tab does. It's still a shell:
comments never go to it. When the shell is back at its prompt, or runs something else, what the
agent held goes, and the tab is a Terminal again.

A shell's tab is named after the program running in it, as VS Code names a terminal's tab:
`cargo` while `cargo test` runs, with a shell's mark (`>_`, `>` on cells) before the name and
the spinner after it, and the shell's own name again, still, once it's back at its prompt. A
command that ends within a second leaves the tab as it was, so a quick `ls` doesn't make it
flicker. The session host owns the PTY, so it reads the process group in front of it
(`tcgetpgrp`), on the half-second look it already took to tell whether the shell is at its
prompt, and while a command is in front, the name of that group's leader (`/proc/<pid>/comm`
on Linux, `proc_name` on macOS), a system call. It reads it on each look rather than once per
group: a group is in front from the moment the shell forks its leader, which has the shell's
own name, or none, until it has started the command, and a name read once in that moment
stood for the whole command. It tells every Cellix attached when the name changes, and
one that joins as it joins (the `f` frame, which a Cellix from before ignores). A shell Cellix
runs itself, as in tests, is read the same way from its own PTY. Windows has no host, and
ConPTY doesn't say what's in front, so there a shell's tab keeps its name.

On Windows the socket is a port on 127.0.0.1, because Rust's standard library has no Unix
sockets there and a named pipe could only be tested on Windows. A file where the socket would
be, in the user's local app data, names the port, two 128-bit tokens and Cellix's process. Each
end proves itself before anything else is said: the client gives its token, and a connection
that doesn't is dropped; Cellix answers with its own, and until it does the client asks nothing,
so whatever takes the port after a Cellix has gone can't pass for it. A note whose process has
ended, or whose port refuses or answers wrongly, is removed, and every request but a plan has a
few seconds at most. It's plain std, so the tests prove it on every platform; Unix keeps its
socket. Windows has no session host yet, so programs in the side pane
run on ConPTY under Cellix and end with it. Cellix sends Windows no terminal queries: keys
arrive as console events, and ConPTY passes on no graphics, so Windows starts on the cell tier
with Cellix's own colours.

## Crates (proposed)

| Crate | Role |
|---|---|
| `cellix-core` | Buffers, undo, workspace, commands |
| `cellix-input` | Simple, Vim and Neovim modes |
| `cellix-render` | Layout, paint, tile cache, software rasterizer |
| `cellix-term` | Probing, Kitty direct, Kitty placeholder and cell backends |
| `cellix-lang` | Tree-sitter, LSP, language registry |
| `cellix-doc` | Typeset math, PDF pages, HTML and LaTeX export, table editing |
| `cellix-review` | Plans, diffs, comments, threads |
| `cellix-agent` | Socket, hooks, IDE server |
| `cellix` | Binary and app views |

## Milestones

0. Finish the rendering proof: the "Not yet proven" list.
1. Render and input skeleton on all three tiers, with file explorer and mouse.
2. Simple-mode editing with highlighting.
3. Plan and diff review with the agent return path. First useful release.
4. LSP, project search, Claude Code plugin.
5. Vim mode, then Neovim mode.
6. The agent loop and the IDE connection.

## Scrolling

A wheel notch used to play a fixed animation that held up the main loop, so a trackpad's stream
of events came through in bursts. Scrolling is now a target and a position. Input moves the target
at once, and the view closes on it on every frame, so a new notch redirects the motion and turning
round never jumps. Claude Code's fullscreen view and vim are the model: no animation to wait for,
one line per event from a trackpad's flood, and speed that ramps only while a wheel keeps spinning.
So the ease is about 14 ms, enough to blend one line into the next and too short to trail the hand. The rows it moves are the whole-row tiles the terminal already
holds, shown through a crop and a pixel offset, so a frame is a line of text per row and nothing is
drawn. The page underneath is already at the end and only catches up now and then, because a typeset
page is expensive to redraw. Typing or a click ends the glide at once.

Tiles are cut into bands at the edges of what moves on its own: the sidebar, the text, the minimap,
the pane. A row's key says nothing of where the row stands. Whole-window tiles tied each row to
whatever the sidebar showed at that height, so every row of a scroll was new, and settling redrew the
window. The periodic look at the tree and git runs on a thread for the same reason: anything on the
main loop shows up as a stall in the glide.

A list in the pane moves by lines, and every notch asks how far the list goes, as every press asks
what's under it and every frame what's in view. For the comments all three used to lay the whole
list out, and wrapping measured a row again each time it grew by a word, so a long list asked for
more strings than the measure keeps (20,000) and shaped every one afresh each time: 0.7 s a notch
for 300 comments, where the loop takes all the notches waiting before it draws. Now a paragraph is
measured once and wrapped from that (`threads::wrap`), and the rows are kept until what they were
laid out from changes (`threads::Laid`, by a hash of the comments, the one being written, the view
and the width, so nothing has to remember to say a comment changed). A notch is 0.4 ms at 300
comments. `harness/guard.sh -- cargo run --release -p cellix --example wheel <project> <file>` times it on any project, and `harness/many_comments.py` makes one with as many comments as asked for.
