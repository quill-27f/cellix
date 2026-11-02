# Cellix design

Date: 2026-11-01. Status: implemented through milestone 4 plus Vim mode. Progress is in
`docs/PROGRESS.md` and `harness/features.json`.

## Goal

Keep people in the terminal they already use, locally or over SSH, and give them
a good-looking, mouse-first editor for reviewing and correcting agent work.

## Decisions

| Topic | Decision |
|---|---|
| Rendering | Pixels in the terminal through the Kitty graphics protocol, with a cell-based fallback |
| Rasterizer | Software, on the host (see below) |
| Agents | The real program on a PTY in a side pane, plus `$VISUAL` and thin plugins. Its interface is never rebuilt |
| Editing | Cellix's simple mode by default, optional Vim and Neovim modes |
| Multiplexers | tmux and Zellij are required targets |
| Scope | Reviewing and correcting agent work, with everyday editing and LSP. No debugger |
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

Detection sends `CSI 16t`, `a=q` and device attributes, and checks `$TMUX` and `$ZELLIJ`.
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

A page uses the pane's full width. A line break inside a paragraph shows as a space that
keeps the newline's source position, so a hard-wrapped file fills the page and saves
unchanged. On a page, arrows and Home and End move by what's shown.

### Companion

Whatever can be seen can be talked about. A selection, in code or on a page, gets Comment and Ask
Agent beside it, and a picture, diagram or formula gets a Comment button under the pointer. Comments
are written in the side pane, with what they're about in view. Ask Agent sends the comment to the
agent as soon as it's written, with its file, line and quote, and how to answer with `cellix reply`,
so the answer lands under it. A picture's comment names its file, so the agent can open it. All of
this works the same in code, Markdown, LaTeX, data and notebooks. A PDF or image in the side pane
takes a comment too, from a button or a right click, and a paper's PDF lands it on the source line
behind that place, so it stays put through every rebuild.

## The pointer

Cellix tells the terminal which pointer to show with OSC 22: a hand on what can be pressed, a bar on
text, an arrow elsewhere. Left to itself a terminal picks one from its own state, which can stick.

## Guidance

Nothing should leave someone wondering what to do next. An empty window lists its actions as rows
that can be clicked, and its first line changes with what's going: with an agent running and no
file open, it says to click a file on the left. The agents panel says how working beside an agent
goes before offering to start one. The first start says how to move between the two sides. Every
panel says what would fill it. Getting Started is a real page, so reading it shows what a page does.

## Documents

A Jupyter notebook opens as a page too, with what each cell printed and drew under it. Its cells
become Markdown with a hidden line above each, and saving puts the text back into the JSON beside
the outputs it already had. Cells run in a real Jupyter kernel, through a small Python program that
passes lines of JSON between Jupyter's client library and Cellix, using the project's own Python if
it has one. Outputs land under each cell as they come, and are saved with the notebook.

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
files stay as they were, because they're an answer the agent is waiting for.

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

## Theme

`theme` is `auto`, `dark` or `light`. `auto` is the default: Cellix asks the terminal for
its background with OSC 11 and takes the matching theme, staying dark if nothing answers.

## Look

The interface follows VS Code's layout and its Dark+ and Light+ colours, because that is
what the target users already read fluently: activity bar, explorer, tabs, breadcrumbs,
blue status bar, current-line highlight, indent guides. It's all drawn to the grid, so it
holds on the cell tier. Activity icons span several cells: shapes on the pixel tier and
block art on cells, because a font glyph is too small beside a real IDE.

Cellix often lives in a pane beside an agent, so width decides the layout. Under 70
columns the activity bar goes, the sidebar becomes a drawer over the code, splits collapse
to the focused pane, the status bar keeps only what fits, and review stacks changes.

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

Requested baseline: the default VS Code experience without extensions.

| v1 | After v1 |
|---|---|
| File explorer with create, rename, delete, drag | Integrated terminal |
| Tabs, splits, command palette, quick open | Notebooks |
| Project search and replace | Remote multi-root workspaces |
| Multi-cursor, tree-sitter highlighting | |
| Git status, side-by-side diff, blame | |
| LSP: go to definition, diagnostics, completion, rename | |
| Markdown preview, settings, themes | |

### Languages

No editor supports literally every language. Support is data-driven: a registry maps
languages to a tree-sitter grammar, with a TextMate grammar as fallback, and to a
language server. Adding one needs no code change. Over SSH, servers run on the
remote host and must be installed there.

### No debugger

Dropped. Agents debug through tests, logs and print statements, and a companion to agents
doesn't need to compete with a full IDE here.

## Agent integration

Each program in the side pane runs under a small host of its own: Cellix re-run as
`cellix session-host`, which owns the PTY and a Unix socket and keeps recent output. So Cellix can
quit, or an SSH connection drop, and the next Cellix in that folder finds it with what it printed.
Quitting offers Keep Running. A dropped connection keeps everything, since nobody can be asked.
Claude Code is told the IDE port when it starts, so Cellix keeps its port and token per folder, and
the next Cellix takes the same ones for an agent that outlived the last.

The side pane's terminal also reads the Kitty graphics protocol, so a program in it can draw
pictures. That is how `terminal-browser` shows a real browser inside Cellix, and how a plot from a
training run shows in a shell. Frames become bitmaps on the `Grid`'s paint list like any other
image, so a terminal without pictures falls back to the system browser.

Cellix runs the real agent and never rebuilds its interface. The side pane is a terminal on a
PTY, using `alacritty_terminal` for the screen, so agents update themselves and nothing here
knows what they look like. Starting one in a host split left the file and the agent in two
programs, and real use showed that isn't side by side. It connects to the real tool in three ways, each proven
against Claude Code 2.1.309:

- **Hooks** reach the workspace socket through `cellix hook <kind>`. They checkpoint turns,
  report status and test runs, and hold plans. Every hook exits cleanly, so a missing
  Cellix never gets in the agent's way.
- **The IDE server** is MCP over a local WebSocket, announced by a lock file in
  `~/.claude/ide`. Only a client with the token gets in, and any request with an Origin is
  refused, which keeps web pages off the port.
- **The mod** adds a status entry and slash commands.

Plans answer on `PermissionRequest`, not `PreToolUse`: a real run showed `PreToolUse` leaves
Claude Code's own dialog up, and the answer must carry `updatedInput` to count.

The plugin ships inside the binary and is written out when an agent starts.

### Inside Claude Code

`cellix pane` runs the whole app with no terminal, started by the mod for the session's
life. It writes each changed frame to a private file and prints one line, and the mod
swaps the pane's image to it. Over SSH the terminal can't read that file, so a frame is a
PNG on stdout. Input comes back through a surface module layered over the image, and each
event is one `cellix pane-send` over the workspace socket. An idle pane sends nothing.

A process per event costs a few milliseconds, which typing doesn't notice. If it shows,
the next step is a long-lived input channel.

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
  only happens when the user asks.
- Codex: a plugin in a local marketplace, installed with Codex's own commands. It runs
  hooks for status, turns, comments and test runs, once the user trusts them with `/hooks`.
- Gemini CLI: an extension with the same hooks, installed with its own command. Its folder
  trust question isn't covered by `--consent`, and the user's click is the answer to it.
- The Agents icon opens the side pane. With nothing running, it lists what's running elsewhere,
  what can be started and the worktrees.
- Rebuilding an agent's interface: the side pane shows the real program, so its features arrive with its updates.

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
