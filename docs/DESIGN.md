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
| Rasterizer | Software first, wgpu when available (see below) |
| Agents | Companion first (pane, `$VISUAL`, thin plugins), hosting agents in a terminal pane later |
| Editing | Cellix's simple mode by default, optional Vim and Neovim modes |
| Multiplexers | tmux and Zellij are required targets |
| v1 | Plan review, diff review, file explorer, everyday editing, LSP |
| Later | Debugging |
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

### GPU

wgpu output has to be read back to the CPU before it can be sent to the terminal,
and one line rasterizes in under a millisecond on the CPU. A remote host often has
no GPU at all. So software rasterizing is built first and is the path most SSH
sessions will use, and wgpu is added behind the same trait where it proves faster.
The on-screen GPU work is done by the user's terminal in every case.

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

Open: tiles are whole rows, so one region can't scroll separately. Smooth pixel scrolling
needs per-region tiles. The Markdown page is still monospaced, because its caret mapping
works in cells, so headings can't yet be larger than body text.

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
| File explorer with create, rename, delete, drag | Debugging (DAP) |
| Tabs, splits, command palette, quick open | Integrated terminal, agent hosting |
| Project search and replace | Notebooks |
| Multi-cursor, tree-sitter highlighting | Remote multi-root workspaces |
| Git status, side-by-side diff, blame | |
| LSP: go to definition, diagnostics, completion, rename | |
| Markdown preview, settings, themes | |

### Languages

No editor supports literally every language. Support is data-driven: a registry maps
languages to a tree-sitter grammar, with a TextMate grammar as fallback, and to a
language server. Adding one needs no code change. Over SSH, servers run on the
remote host and must be installed there.

### Debugging later: validated, with a condition

Deferring is sound. Agents already debug through tests, logs and print statements,
which covers most day-to-day work. Interactive debugging still matters for race
conditions, memory corruption and state that is hard to reproduce, so it is deferred
rather than dropped. The LSP client's process and transport layer should be reusable
for DAP. A later DAP integration could also let agents drive the debugger.

## Agent integration

Cellix runs one local socket per workspace, and each integration is a thin adapter.
Comments reach an agent through `cellix comments [--take]` as JSON. With `--wait` they are
also appended to the file as a "Review comments" section, because `$VISUAL` only returns
the file.

- `$VISUAL`: `cellix --wait <file>` covers Ctrl+G in Claude Code and equivalents elsewhere.
- Review return path: comments are delivered as structured data over the socket and
  CLI, so any agent can consume them.
- Claude Code: a plugin with a mod for status and commands. The mod is a bridge, not
  the editor: docked panes stop at 45% width and there is no terminal element.
- Codex and Gemini CLI: `$VISUAL` plus skills or extensions. Verify current hooks
  before building each adapter.
- ACP: consider when agent hosting begins.

## Crates (proposed)

| Crate | Role |
|---|---|
| `cellix-core` | Buffers, undo, workspace, commands |
| `cellix-input` | Simple, Vim and Neovim modes |
| `cellix-render` | Layout, tile cache, software and wgpu rasterizers |
| `cellix-term` | Probing, Kitty direct, Kitty placeholder and cell backends |
| `cellix-lang` | Tree-sitter, LSP, language registry |
| `cellix-review` | Plans, diffs, comments |
| `cellix-agent` | Socket, CLI, adapters |
| `cellix` | Binary and app views |

## Milestones

0. Finish the rendering proof: the "Not yet proven" list.
1. Render and input skeleton on all three tiers, with file explorer and mouse.
2. Simple-mode editing with highlighting.
3. Plan and diff review with the agent return path. First useful release.
4. LSP, project search, Claude Code plugin.
5. Vim mode, then Neovim mode, then agent hosting, then debugging.
