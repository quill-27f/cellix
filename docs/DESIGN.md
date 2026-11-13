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

Detection sends `CSI 16t`, `a=q` and device attributes, and checks `$TMUX` and `$ZELLIJ`. The background colour
(`OSC 11`) decides light or dark, and is asked again each time the window regains focus, since a
terminal follows the system's appearance. tmux answers that question from what it learned when
the client attached and keeps the terminal's own answer to a passthrough question to itself, but
the passthrough question is how it learns the new colour, so inside tmux Cellix asks past it
first, waits for the cell size answered after it, then asks tmux.
tmux swallows the replies, so under tmux Cellix asks tmux for the client terminal name and
`allow-passthrough`. `CELLIX_TIER` can force a tier. `CSI 16t` is preferred for cell size
because the window size call was not reliable under Zellij at startup. Window size is
polled as well as signalled, because a resize during font loading is otherwise lost.

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

If 1 or 2 fail, cells become the default in that environment.

## Paint layer

Cells alone read as an ASCII drawing, even as pixels. So the grid stays the source of truth
for layout, hit-testing and the cell tier, and a list of paint items sits above it for the
pixel tiers: proportional text, rounded panels, hairlines, shadows, icons and a thin caret.
Paint positions are in 64ths of a cell, so they can sit between cells. A row's tile hashes
the paint that touches it, so caching and reuse work as before. Cell text marked `painted`
is skipped on pixel tiers and kept for the cell tier and tests.

Fonts are embedded (Inter, JetBrains Mono), because pixels are drawn on the remote host
over SSH and servers rarely have fonts.

Text is measured with the same fonts the rasterizer draws, through the `Measure` trait, so
carets and wrapping use real glyph positions. The cell tier measures one column per
character. Where the terminal offers mode 1016 the mouse reports pixels, because a cell
can hold two thin letters.

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
sideways inside the page, and a letter that would be cut in half at its edge is left out.
The middle is that of the columns the page has: the pane's, less its minimap's when one is
showing, so a long page isn't pushed against its minimap. A page is laid out before that's
decided, so its first frame is where it stays. A pane no wider than the page sets it as
before, from its own margin, with blocks running into the right margin. A sheet of data (CSV, TSV) keeps the whole pane,
since it's read across. `reading_width: false`, or Reading width under Settings, Editor, lets
every page run the width of its pane, as all pages did from 1 to 10 November 2026 by the
user's choice then; the user chose the reading width after a day in a wide window. A line
break inside a paragraph shows as a space that keeps the newline's source position, so a
hard-wrapped file fills the page and saves unchanged. On a page, arrows and Home and End move by what's shown.

### Companion

Whatever can be seen can be talked about. A selection, in code or on a page, gets Comment and Ask
Agent beside it, and a picture, diagram or formula gets a Comment button under the pointer. Comments
are written in the side pane, with what they're about in view. Ask Agent sends the comment to the
agent as soon as it's written, with its file, line and all of what was selected, and how to answer
with `cellix reply`, so the answer lands under it. Inside Claude Code it puts the file and the whole
selection in Claude Code's prompt instead, a passage set out as a quote. A comment is typed as any
text box is, with a caret that moves by letter and word and the Mac's word deletes. Resolving one
stops marking its words at once; the pane can show resolved comments again, to reopen them. A picture's comment names its file, so the agent can open it. All of
this works the same in code, Markdown, LaTeX, data and notebooks. A PDF or image in the side pane
takes a comment too, from a button or a right click, and a paper's PDF lands it on the source line
behind that place, so it stays put through every rebuild.

Output is seen too, so a failing run needs no copy and paste. A selection in a terminal tab gets the
same two buttons, where they cover none of the output: on the row above the selection when its text
ends before them, else on the row below, their right edge on the selection's, and failing both after
the end of the selection's last row. Only output with no such room has them over the end of the row
above. The agent hears all of the output, the command that printed it and every
`file:line` in it that is a file of the project. Cellix knows the command from where the first key
was typed while the shell was at its prompt, read off the screen when it's asked. A comment on output
has no file, so it shows in the Comments tab whatever file is in view, and opening it goes to the
first place it names. A row of the Problems tab gets Ask Agent under the pointer, which sends what the
checker said, the file and line, and three lines of code either side. With no file in view the tab lists every file's
comments, so each file's stand under a heading with its icon, its name and how many, as in VS Code's
Comments view: a file at a time by name, a terminal's output last. A click on a heading opens the
file, and the list is then that file's alone.

## The pointer

Cellix tells the terminal which pointer to show with OSC 22: a hand on what can be pressed, a bar on
text, an arrow elsewhere. Left to itself a terminal picks one from its own state, which can stick.

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
the file, and it reaches the web page and the PDF. Track Changes keeps edits as suggestions in the
text itself, as CriticMarkup or the `changes` package, so they travel with the file and any tool,
agents included, can read them.

The Outline in the sidebar lists a document's headings, or a paper's sections, and a click goes
there. The side pane keeps what the last build said in a Log tab, and a line that names a place
goes to it. Wide glyphs are looked up as sans-serif rather than monospace in the cells: asked for a
monospaced Han glyph, the font search stops at a font that holds only blanks.

Diagrams are text in the document. A fenced `mermaid`, `dot` or `svg` block stands on the page as
a picture, and opens to its source with the picture under it while it's written. They are drawn by
pure Rust (`mermaid-rs-renderer`, `layout-rs`, `resvg`), so nothing needs a browser or Node, and
the web page and PDF export carry them too. What only LaTeX can draw (TikZ, `tikzcd`, `forest`,
`circuitikz`, `algorithm`) is built alone in the background with the paper's own engine and setup,
and kept by its content in the cache folder, so typing never waits. Line art becomes ink in the
sheet's colour, so it reads in both themes. An image's size is written as `{width=50%}`, which
Pandoc reads as well, and as `width=0.5\linewidth` in a paper. Dragging an image's edge sets the
same thing, in steps of five percent.

Markdown and LaTeX share one page: `rendered.rs` lays out Markdown and `rendered/tex.rs` lays out
LaTeX, into the same rows of glyphs that each remember their source position. So clicks, selection
and editing work the same in both, and the file is always the document.

LaTeX on the page is a reading of the source, not a compiler. Commands it knows hide, and anything
else shows as source, so a paper can never lose text to the page. Numbers come from a first pass
over the labels, as LaTeX's own does. The PDF is the truth for how a paper prints.

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

## Surfaces

The rail and the status bar are the frame, in the terminal's own colour, so window padding
can't show as a seam around them. The sidebar is raised from the frame, and the sheet the
work sits on is raised again. Every surface moves together when the terminal's background
does, so the steps between them hold on any colour.

Paint can't go under text, so a rounded surface is coloured cells with its corners painted
back in the colour behind. Icon pills work the same way. Floating panels reach past their
rows in paint, which gives them padding that a row of cells couldn't.

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

The commit graph sits at the end of Source Control, as in VS Code. Its lanes are straight
lines with square joins, box-drawing characters on cells and hairlines on pixels, and a merge
is a hollow dot. A commit opens to its files under it; a file opens the diff view against the
commit's first parent. The open file's commits are the Explorer's Timeline.

Commits, stashes and Compare With use the diff view read-only: no Accept or Reject, a list of
what's in them, and a title naming both sides. A stash is named by its commit, never its
place in the list, since every worktree shares one list and agents push and pop it.

## Theme

`theme` is `auto`, `dark` or `light`. `auto` is the default: Cellix asks the terminal for
its background with OSC 11 and takes the matching theme, staying dark if nothing answers.

## Look

The interface follows VS Code's layout, because that is what the target users already read
fluently: activity bar, explorer, tabs, breadcrumbs, current-line highlight, indent guides.
Both themes are three surfaces a step apart: the frame (the rail, the tab strips, the status
bar), the panels (the sidebar and the side pane, whatever it shows, a terminal too) and the
sheet (the file). Cards, menus and dialogs are raised over those, and a text box is sunk. Text
has three weights of colour and there is one blue. The dark theme has deep blue-grey surfaces
of its own with Catppuccin Mocha's colours for code and for meaning (green added, peach
changed or waiting, red removed or failed). The light one is Catppuccin Latte's surfaces, the
editor on base, the sidebars on mantle, the frame on crust and menus white; its text and code
colours are Latte's hues deepened until each reads at 4.5 to 1, which Latte's own green,
yellow and blue don't (the end-to-end design the user approved on 2026-11-12, which replaced
"Latte's own colours for code" of 2026-11-09).
On a terminal, both themes move together so the frame sits on the terminal's own background and
its window padding shows no seam; a Latte terminal therefore shows the light theme a step paler
than Latte (decided with the user on 2026-11-09). It's all drawn to the grid, so it holds on the cell tier. Activity icons span several cells:
shapes on the pixel tier and block art on cells, because a font glyph is too small beside a
real IDE. The highlight behind a selected or hovered icon, on the rail and at the top right,
keeps 8 px or more from the icon on every side; the rail's reaches into the rows either side,
so the rail sits an eighth of a row lower to keep the first one inside the window.

One set of parts, everywhere. A button is one chip, nearly nine tenths of its row and inside it
(four fifths left its words 7 px under its top, which read as cramped): the
primary one filled with the accent, the rest a quiet fill, the least of them plain words (Edit and
Delete on a comment), and each takes a click anywhere it's painted, from one rectangle the painter
and the hit test share. One measure sizes them all (`measures.rs`): a button's words keep a column
and a half either side inside its chip, whatever they are, and buttons side by side stand two
columns apart. In cells a button is its words between brackets. Text stands two columns inside
whatever holds it, a panel, a card, a menu, a prompt; a card starts one column from its pane's
edge and is raised over it, as a menu is over the page. A bar that holds a text box, the Find bar
and a prompt, keeps a quarter of a row clear above and below its rows, which only paint can show,
and a box's text starts a column inside it. A section's heading is small, bold, quiet capitals
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
line along its top. One behind is its name, brighter under the pointer. Overlays sit on row lines: a menu
is whole rows, padded 8 px above and below, with half a row for each rule and its hover band
clear of them, and it drops from the bottom edge of what opened it; a tooltip is one row, on
the row under its icon. Nothing covers part of a row.

Git has two places. Source Control in the sidebar decides what goes in: a message, staged and
unstaged files, Commit, and Push or Pull, as in VS Code. Review reads what changed, hunk by hunk,
and takes over the explorer's width unless Source Control is showing, in which case it stands
beside the panel with no list of its own.

Cellix often lives in a pane beside an agent, so width decides the layout. Under 70
columns the activity bar goes, the sidebar becomes a drawer over the code, splits collapse
to the focused pane, the status bar keeps only what fits, and review stacks changes.
Under 80 columns, about what Claude Code leaves Cellix beside its chat, the layout is made
for that width rather than squeezed. The five panel icons lead the side pane's header, as
VS Code's panel shows its views, with the one showing named and no tab of its own. The top
right keeps one icon that shows or hides the pane, beside quit. Tabs out of view are
counted at the ends of the strip ("‹ 2 more", "3 more ›"), and each count lists its tabs.
A side pane whose tabs don't all fit before the icons at the top right keeps every tab in
the row. The one in front, a tab in the shape every tab has, hugs its measured name and its ×, with none
of the slack a tab has when there's room, since that slack is what would cut its neighbours.
Those behind it stand as names alone, half a column of air either side, whole while they
fit and then cut short with a "…", the longest giving way first. The + gives way too, since
the icons beside it start the same things. A tab behind has no ×, so a click anywhere on it
brings it forward, and a cut name is a hover away. Only when that would leave them under
three letters and the "…" each (five columns) does the row show the tab in front alone, with
a chevron that lists every tab and what + would start: two letters don't tell Comments from
Codex. The icons stand three columns
apart on their row's own ground, and nothing runs under them.
The page's menu folds formatting and inserting into Format › and Insert ›, and the agents
panel gives each agent one row under two lines of how-to. At every width a page keeps a
cell for a comment's ◆ and one for a change's bar, and a long page scrolls past its end
until its last row is a third of the way down.
The side pane or one editor can be maximized, from the pane's own tab row or a double-click on
a tab. The side pane takes the keyboard as it takes the window, as a click on it would, when it
shows a session or the Preview. The
icons at the top right never move for it. Whatever asks for a
hidden part, such as one of those icons or opening a file, restores first, so nothing opens
out of sight. A pane hidden while it has the window gives the window to the file, and takes
it again when it's shown, by its button in the corner or any other way: it comes back as it
was, as VS Code's maximized panel does. The side pane stands under the file when it's dragged there, as well as when the
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
cells. Brackets take one of three colours by depth, gold, orchid and sky as in VS Code, toned for
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

A host that dies without finishing, as when the machine restarts, leaves its job, log and session
id; the next Cellix puts them aside and shows the tab as it was, with Resume (Claude Code's
`--resume`, Codex's `resume`), Start Fresh or Start Again, and Close, and with others stopped too,
Resume All. It says "restarted" only when the machine booted after the log was last written.

### How the agent got there

Reviewing a result isn't only its diff: the steps, commands and reasoning that led to it can be
wrong too. An agent's hooks leave its transcript's path beside its socket with its session id, and
send it with every report, so Cellix knows each tab's transcript: Claude Code's
`~/.claude/projects/<project>/<session>.jsonl`, or Codex's rollout log, found by the session id when
a hook didn't say. Right-click an agent's tab, or run Open Agent Transcript, and it opens as a page
beside the code, made by Cellix into `.cellix/transcripts` (ignored by git) and read only: each turn
under a heading with how its context grew, the prompt in a callout, the replies as written, and
each tool call on one line with its input and a word on the result. A command's output, and any
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
give). The picker moves between Claude Code, Codex and Gemini CLI. All of it is read as data.

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
agent's context grew over it, from that transcript.

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
names its project, so a new Cellix opens every folder on the machine with something running or
stopped that no other Cellix has open, and Cellix started at home with nothing named offers them
first. That first screen can be left as any other: Quit beside Open, the × in its corner, or the key
that quits, and every folder stays as it is for next time, with nothing asked. Every folder open when Cellix quits or hangs up is marked `open` there too, so it comes
back with nothing running in it, as an editor brings back its windows; closing its tile lets it
go. Closing a tile asks as Quit does; Quit asks each folder in turn. The bar is drawn on the
same grid as everything else, so it works on the cell tier too.

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
knows what they look like. When a program there ends, as an agent does on `/exit`, its tab
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
Comments reach an agent through `cellix comments [--take]` as JSON. With `--wait` they are
also appended to the file as a "Review comments" section, because `$VISUAL` only returns
the file.

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
- The icons at the top right, as in a desktop app, reach the comments, a terminal, Review, the
  browser and the agents. Past them, in the corner, a close button quits Cellix as Ctrl+Q
  does. It's drawn as the icons are, grey with the same hover chip, and a thin line sets it
  apart from the pane's own control, which hides the pane and is drawn as a window with its
  right part filled, so the only × in the corner quits. A hidden pane leaves that control in
  its place, the window with its right part in outline, so something in view always brings
  the pane back as it was. The
  explorer lists gitignored files, as VS Code does, since agents work in them, but dims them,
  and what's dimmed is exactly what Quick Open and search leave out. The pane stands under the file when the window is too narrow to have
  them side by side. The Agents icon opens the side pane. With nothing running, it lists what's running elsewhere,
  what can be started and the worktrees.
- Rebuilding an agent's interface: the side pane shows the real program, so its features arrive with its updates.

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
