# Cellix design

Date: 2026-11-01. Status: draft, based on `spikes/pixel-budget`.

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
| Cells | Anything else, or a host with no fonts or no known cell size | Styled text cells |

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

Static pages only have been verified. These decide whether pixels stay the default:

1. Live scrolling smoothness in Ghostty, tmux and Zellij. The 5,967 fps from `demo`
   is the rate the terminal accepted bytes, not frames displayed.
2. tmux and Zellij behaviour with split panes, pane switching, resizing and detach.
3. A real SSH session on a slow link.
4. Other terminals: Kitty, WezTerm, iTerm2, and whether Zellij behaves the same on them.
5. Pixel mouse reporting (mode 1016) through Zellij.

If 1 or 2 fail, cells become the default in that environment.

## Modes

A single input layer maps keys to editor commands.

- **Simple (default):** VS Code-style keys, mouse for everything, no modes to learn.
- **Vim:** built-in emulation, because Vim has no embedding interface.
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
| `cellix` | Binary |

## Milestones

0. Finish the rendering proof: the "Not yet proven" list.
1. Render and input skeleton on all three tiers, with file explorer and mouse.
2. Simple-mode editing with highlighting.
3. Plan and diff review with the agent return path. First useful release.
4. LSP, project search, Claude Code plugin.
5. Vim mode, then Neovim mode, then agent hosting, then debugging.
