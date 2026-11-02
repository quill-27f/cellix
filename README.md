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

Typing suggests as you go in every file type, with no setup: keywords, words from open
files, and in `.env` files the keys from sibling files such as `.env.example`. A language
server joins in where one is installed: `rust-analyzer`, `terraform-ls`, `gopls`,
`pyright-langserver`, `typescript-language-server`, `yaml-language-server`, `taplo`,
`bash-language-server` and `clangd`. Shift+F12 lists a symbol's references, and Alt+Shift+F formats the file, with its language server or a formatter such as `rustfmt` or `prettier`. Tab accepts, and so does Enter outside Markdown.
Brackets and quotes close themselves, Ctrl+/ toggles comments, Alt+Up and Down move lines,
Alt+Shift+Down duplicates one, and Ctrl+L goes to a line.

The margin marks lines changed since the last commit, and the minimap fits the whole file,
so you can see how long it is. Click or drag it to move.

Mac text shortcuts work: Option+Backspace deletes a word, Option+Left and Right move by
word, and Cmd+Backspace deletes to the start of the line. tmux doesn't pass Cmd through, so
for Cmd+Z there, add `keybind = cmd+z=text:\x1a` to your Ghostty config.

Cmd works as Ctrl where the terminal passes it through. Ctrl+Shift+P needs a terminal with
the Kitty keyboard protocol, such as Ghostty, Kitty or WezTerm. Ghostty keeps Cmd+Shift+P
for its own palette unless you rebind it.

Markdown opens as one editable page, as in Xcode: the markers are hidden and you type on
the rendered text. Paragraphs flow across the whole pane, and the file keeps its own line
breaks. The toolbar has bold, italic, code, heading, list, task and quote, and
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

`Open in Browser` in the palette, or Ctrl+click on a link, opens the link at the cursor, the
current HTML file, or a Markdown file as a page of its own, with its math typeset. Set
`browser` in the settings file to use another command first, such as
[terminal-browser](https://github.com/zenbu-labs/terminal-browser). If that fails, the
system's browser takes over.

## Inside Claude Code

Cellix can run as a pane in Claude Code, so you start Claude and Cellix comes to you:

```sh
cellix setup --install    # or Install under Settings, Integrations
```

That adds two lines to the `env` block of `~/.claude/settings.json`, which load the plugin
in every session and connect to a running Cellix, so there are no flags to remember. It
keeps everything else in the file, and `cellix setup --remove` takes the two lines back.
Restart Claude Code to pick it up. To try it in one session instead, `cellix setup` prints
the command.

Then `/cellix` opens the pane, and `/cellix-open <file>` opens a file in it. It also opens
by itself when Claude proposes an edit or a plan. Click the pane to type there, and press
Esc to go back to Claude. Under 110 columns the pane drops the sidebar, and under 90 the
minimap, so the code keeps the room.

- **Proposed edits** open in the diff view. Accept or reject them by hunk or whole, in
  Cellix or in Claude's own prompt.
- **Your selection** shows up in Claude's prompt, and `Send Selection to Agent` mentions it.
- **Plans** open as a page with Approve and Send back. Your edits and comments go with the
  answer, and this replaces Claude's own plan dialog.
- **Comments** go with `Send Comments to Agent`, or with your next prompt.
- **Turns** are checkpointed. Review shows what each prompt changed, and `Undo Agent Turn`
  puts the tree back, keeping a checkpoint of what it replaced.
- **Agents** (the robot icon) is what's happening here: who's running in this folder, who
  needs you, and every worktree. Agents started without a plugin still show up, found by
  their working folder. Agents that are installed can be started from the bottom of it.
- **Tests** an agent runs appear in review with their results.
- **Staging**: Accept stages the hunk in git, and the Commit button commits what's staged.

The pane needs a terminal with Kitty graphics, such as Ghostty, Kitty or WezTerm. Over SSH
each update travels as a PNG, so it costs more than the standalone app on a slow link.

## Beside an agent

Running Cellix on its own inside tmux or Zellij also works: `Open Claude Code` in the
palette starts the real tool in a split, connected to Cellix. `Open Codex` starts Codex
there without these connections.

Set `VISUAL="cellix --wait"` so Ctrl+G in Claude Code opens plans in Cellix. Comments are
written back into the file when you quit. Any other agent can run `cellix comments --take`
for your comments as JSON, or `cellix open <file>` to show you something.

The IDE connection follows what Claude Code's VS Code extension does, which isn't a
published protocol, so a Claude Code update could change it. It was built against 2.1.309.

### Codex and Gemini CLI

Settings, Integrations has an Install button for each, or the plugin's source is under
`integrations/`. They report what the agent is doing, record a turn for each prompt, show
test runs in review, and hand the agent your comments. They don't get the pane or plan
review, which rely on what only Claude Code offers.

Codex asks you to trust a plugin's hooks before running them, so run `/hooks` in Codex once
after installing. Restart Gemini CLI to load its extension. Google now points people to
Antigravity CLI in place of Gemini CLI, and that isn't covered here yet.

## Settings

The gear opens Settings: theme, smooth scrolling, the minimap, the keymap, language servers
with a one-click install for each, and the installs for your agents. Changes apply as you click, and are
saved to `~/.config/cellix/settings.json`. You can also edit that file, or
`.cellix/settings.json` in a workspace:

```json
{ "keymap": "vim", "theme": "light", "tier": "cells" }
```

`keymap` is `simple`, `vim` or `neovim`, which runs your own `nvim` and config behind the
editor. `theme` is `auto`, `dark` or `light`, and `auto` also matches your terminal's
background. `tier` is `direct`, `placeholder` or `cells`. `smooth_scroll: false` makes the
wheel jump by lines, for a slow link, and `minimap: false` hides the minimap.
`language_servers` maps a language to a command, and
`browser` replaces the command that opens web pages.

Ghostty keeps Cmd+D to open a split, so Cellix never sees it and Ctrl+D is the way to select
the next match. `Free Cmd+D` under Integrations adds `keybind = super+d=unbind` to Ghostty's
config, which then needs a reload.

tmux needs `set -g allow-passthrough on` for pixels, as shown under [Over SSH](#over-ssh).
Without it, Cellix uses text cells.

See [docs/DESIGN.md](docs/DESIGN.md) for the design and [AGENTS.md](AGENTS.md) for how to
work on it.

Cellix embeds [Inter](crates/cellix-render/fonts/LICENSE-Inter.txt) and
[JetBrains Mono](crates/cellix-render/fonts/LICENSE-JetBrainsMono.txt), both under the SIL
Open Font License.
