# Cellix

A terminal-native editor for agentic development: review what the agent planned and
changed, correct it, and keep working without leaving your terminal, even over SSH.

The name joins cell and pixel: it renders pixels in the terminal and falls back to text
cells.

Status: the full feature set is built and tested, and it hasn't yet been used over a real
SSH link. See [docs/PROGRESS.md](docs/PROGRESS.md) for what has and hasn't been proven.

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
| Ctrl+F | Find in the file |
| Ctrl+Shift+F | Search the project |
| Ctrl+\\ | Split pane |
| F12, F2, Ctrl+Space | Definition, rename, completion |
| Ctrl+S, Ctrl+Q | Save, quit |

Ctrl+W closes a tab and Ctrl+Q quits. Ghostty keeps Cmd+W and Cmd+Q for itself, so they
only reach Cellix if you unbind them with `keybind = cmd+w=unbind` and
`keybind = cmd+q=unbind` in your Ghostty config.

Mac text shortcuts work: Option+Backspace deletes a word, Option+Left and Right move by
word, and Cmd+Backspace deletes to the start of the line. tmux doesn't pass Cmd through, so
for Cmd+Z there, add `keybind = cmd+z=text:\x1a` to your Ghostty config.

Cmd works as Ctrl where the terminal passes it through. Ctrl+Shift+P needs a terminal with
the Kitty keyboard protocol, such as Ghostty, Kitty or WezTerm. Ghostty keeps Cmd+Shift+P
for its own palette unless you rebind it.

Markdown opens as one editable page, as in Xcode: the markers are hidden and you type on
the rendered text. The toolbar has bold, italic, code, heading, list, task and quote, and
Toggle Markdown Source in the palette shows the raw file.

Under 70 columns, as beside an agent, the sidebar becomes a drawer behind the `☰` button
and review stacks changes in one column.

The mouse works everywhere: click files, tabs and hunks, drag to select, and right-click
the explorer to create, rename or delete.

## Over SSH

Everything runs on the server, and your local terminal only draws the result. This setup
hasn't been tried on a real SSH link yet. On the server, you need Rust, git and tmux, plus
Claude Code if you want the agent features:

```sh
git clone https://github.com/quill-27f/cellix.git
cd cellix
cargo install --path crates/cellix

# Pixels through tmux, no focus warning from Claude Code, click to switch panes
cat >> ~/.tmux.conf <<'CONF'
set -g allow-passthrough on
set -g focus-events on
set -g mouse on
CONF
tmux source-file ~/.tmux.conf 2>/dev/null   # only needed if tmux is already running

# Language server for Rust projects, optional
rustup component add rust-analyzer
```

Then, from your own machine:

```sh
ssh you@your-server
tmux new -A -s work   # starts the session, or rejoins it after a dropped connection
cd your-project
cellix
```

Inside Cellix, press Ctrl+Shift+P or F1 and choose `Open Claude Code`. It opens in a tmux
split, already connected to Cellix.

If pixels look wrong or scrolling is heavy, these commands help:

```sh
tmux show -g allow-passthrough   # should say on
CELLIX_TIER=cells cellix         # text cells only, for one run
```

To turn off smooth scrolling on a slow link, add this to `~/.config/cellix/settings.json`:

```json
{ "smooth_scroll": false }
```

## The web

`Open in Browser` in the palette, or Ctrl+click on a link, opens the link at the cursor or
the current HTML file. It uses [terminal-browser](https://github.com/zenbu-labs/terminal-browser)
in a split when that's installed, and your system browser otherwise.

## With an agent

Run Cellix inside tmux or Zellij, then choose `Open Claude Code` in the palette. It starts
the real Claude Code in a split, connected to Cellix as its IDE, with the Cellix plugin
loaded. There's nothing to install, including on a server. Then:

- **Proposed edits** open in Cellix's diff view. Accept or reject them by hunk or whole, in
  Cellix or in Claude's own prompt.
- **Your selection** shows up in Claude's prompt, and `Send Selection to Agent` mentions it.
- **Plans** open as a rendered page with Approve and Send back. Your edits and comments go
  with the answer, and this replaces Claude's own plan dialog.
- **Comments** go to the agent with `Send Comments to Agent`, or with your next prompt.
- **Turns** are checkpointed. Review shows what each prompt changed, and `Undo Agent Turn`
  puts the tree back, keeping a checkpoint of what it replaced.
- **Agents** (the robot icon) lists who's running and who needs you, and every worktree.
- **Tests** an agent runs appear in review with their results.
- **Staging**: Accept stages the hunk in git, and the Commit button commits what's staged.

Outside tmux or Zellij, start Claude Code yourself with
`claude --plugin-dir ~/.local/share/cellix/claude-code` after using the palette once, and
pick Cellix under `/ide`. `Open Codex` starts Codex in a split, without these connections.

Set `VISUAL="cellix --wait"` so Ctrl+G in Claude Code opens plans in Cellix. Comments are
written back into the file when you quit. Any other agent can run `cellix comments --take`
for your comments as JSON, or `cellix open <file>` to show you something.

The IDE connection follows what Claude Code's VS Code extension does, which isn't a
published protocol, so a Claude Code update could change it. It was built against 2.1.309.

## Settings

`.cellix/settings.json` in a workspace, or `~/.config/cellix/settings.json`:

```json
{ "keymap": "vim", "theme": "light", "tier": "cells" }
```

`keymap` is `simple`, `vim` or `neovim`, which runs your own `nvim` and config behind the
editor. `theme` is `auto`, `dark` or `light`, and `auto` also matches your terminal's
background. `tier` is `direct`, `placeholder` or `cells`. `smooth_scroll: false` makes the
wheel jump by lines, for a slow link. `language_servers` maps a language to a command, and
`browser` replaces the command that opens web pages.

tmux needs `set -g allow-passthrough on` for pixels, as shown under [Over SSH](#over-ssh).
Without it, Cellix uses text cells.

See [docs/DESIGN.md](docs/DESIGN.md) for the design and [AGENTS.md](AGENTS.md) for how to
work on it.

Cellix embeds [Inter](crates/cellix-render/fonts/LICENSE-Inter.txt) and
[JetBrains Mono](crates/cellix-render/fonts/LICENSE-JetBrainsMono.txt), both under the SIL
Open Font License.
