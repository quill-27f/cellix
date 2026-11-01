# Cellix

A terminal-native editor for agentic development: review what the agent planned and
changed, correct it, and keep working without leaving your terminal, even over SSH.

The name joins cell and pixel: it renders pixels in the terminal and falls back to text
cells.

Status: early. Plan and diff review, everyday editing and Vim mode work and are tested.
Definition, rename, completion and compiler errors work with rust-analyzer, and no other
language server has been tried. See [docs/PROGRESS.md](docs/PROGRESS.md).

## Try it

```sh
cargo install --path crates/cellix
cellix            # the current folder
cellix src/lib.rs # with a file open
```

| Keys | Action |
|---|---|
| Ctrl+P | Go to a file |
| Ctrl+Shift+P, F1 or Alt+P | Command palette |
| Ctrl+G | Review changes against HEAD |
| Alt+C, or right-click | Comment on a line |
| Ctrl+Shift+F or Ctrl+F | Search the project |
| Ctrl+\\ | Split pane |
| F12, F2, Ctrl+Space | Definition, rename, completion |
| Ctrl+S, Ctrl+Q | Save, quit |

Cmd works as Ctrl where the terminal passes it through. Ctrl+Shift+P needs a terminal with
the Kitty keyboard protocol, such as Ghostty, Kitty or WezTerm. Ghostty keeps Cmd+Shift+P
for its own palette unless you rebind it.

Markdown opens as one editable page, as in Xcode: the markers are hidden and you type on
the rendered text. Toggle Markdown Source in the palette shows the raw file.

Under 70 columns, as beside an agent, the sidebar becomes a drawer behind the `☰` button
and review stacks changes in one column.

The mouse works everywhere: click files, tabs and hunks, drag to select, and right-click
the explorer to create, rename or delete.

## With an agent

Set `VISUAL="cellix --wait"` so Ctrl+G in Claude Code opens plans in Cellix. Comments are
written back into the file when you quit.

While Cellix is open, any agent can run `cellix comments --take` for your comments as JSON,
or `cellix open <file>` to show you something. `integrations/claude-code` is a Claude Code
plugin that teaches this.

## Settings

`.cellix/settings.json` in a workspace, or `~/.config/cellix/settings.json`:

```json
{ "keymap": "vim", "theme": "light", "tier": "cells" }
```

`keymap` is `simple` or `vim`, `theme` is `dark` or `light`, and `tier` is `direct`,
`placeholder` or `cells`. `language_servers` maps a language to a command.

tmux needs `set -g allow-passthrough on` for pixels. Without it, Cellix uses text cells.

See [docs/DESIGN.md](docs/DESIGN.md) for the design and [AGENTS.md](AGENTS.md) for how to
work on it.
