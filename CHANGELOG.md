# Changelog

All notable changes to Cellix. Versions follow semver; each one is on the [releases page](https://github.com/quill-27f/cellix/releases).

## [Unreleased]

## [0.6.4] - 2026-11-10

### Added
- Paste a picture from your own machine into Cellix over SSH, in Ghostty 1.4 or kitty 0.44 and later, outside tmux and Zellij. Cellix asks the terminal for it with Cmd+V, saves it in `.cellix/pasted` and pastes its path; on a Markdown page it goes into `images/`. `CELLIX_PASTE_EVENTS=0` turns it off.

### Changed
- The side pane's width, its height under the file and full screen come back next time.
- A pane that takes the window also takes the keyboard when it shows a session or the Preview.
- An agent's tab closes when its session ends. One that ends within 5 seconds of starting stays, to show why.
- A reopened session starts on the pane tab that was in front, not on Terminal.
- The status bar's bell has air above it, and selected icons have air inside their highlight.

## [0.6.3] - 2026-11-09

### Added
- Edit and Delete on your own comments: Edit changes the text in place, Delete asks once.

### Changed
- Review Changes opens beside Source Control, which stays on the left, with its own list of files, turns and tests. An × after its title, or Esc, closes it. The ± is gone from the top right, where every icon now opens the side pane. Where Review takes Source Control's place in a narrow window, Source Control's icon goes back to it.
- Review's cards have air around their header and buttons, Accept as far in from the right as the title is from the left, and Reject in the secondary button style used elsewhere.
- The workspaces bar has room under it at the window's edge.

### Fixed
- A terminal test that read the workspaces bar before it was drawn.

## [0.6.2] - 2026-11-09

### Changed
- The light theme sets its frame on the terminal's background, as the dark theme does, so a light terminal's window padding no longer shows as a paler strip beside the activity bar. On a Latte terminal the sidebar and editor are a shade paler than Latte. Settings' theme cards show the theme as it sits on your terminal.

### Fixed
- Menu highlights, text boxes, the toolbar's active button and code blocks keep their contrast on the paler light surfaces.
- Two agents taking checkpoints at once on a busy machine could still lose one.

## [0.6.1] - 2026-11-09

A review from the side of someone building on coding agents, of how Cellix talks to them and what
it reads from them, and the polish left from 0.6.0. There's no Windows build of this release:
Windows builds and CI are paused while no one tests there.

### Fixed
- The context meter took a 200k window for every Claude model, so Opus 5.5 at 170k read 85% and "compacts soon". Every current Claude model has a million; only Claude 4.5 and older have 200k.
- Control characters in an agent's output, a transcript or a file reached the terminal, where an escape could set its title or write the clipboard. They show as symbols such as ␛.
- Text Cellix types into an agent, from a file or a terminal, could end the paste early and arrive as keys, which in Claude Code can run a shell command. Control characters are dropped from it.
- Quoted code or output in a comment could close its quotes and read as your instruction to the agent. It's quoted so it can't.
- An agent could reply to a comment as "You".
- On a shared machine, another user could make Cellix's sockets folder first and answer your agents' hooks. A folder that isn't yours and private is refused.
- A plan path from Gemini CLI was read and written without a check, so any file could be named; an edited Claude Code plan now also reaches its own plan file.
- A failed test run in Codex or Gemini CLI could show as passed.
- After an interrupt or an API error an agent stayed "working", and an answered question still counted as needing you. The Claude Code plugin now hooks `StopFailure`.
- A second agent's plan replaced the first one waiting; it now waits its turn. Gemini CLI's news reaches its tab.
- Two agents prompting at once in one checkout could overwrite each other's turn checkpoint.
- An agent run by another, such as `claude -p` from its shell, took over the tab's session to resume.
- Agent Context follows Claude Code's current rules: AGENTS.md when there's no CLAUDE.md, rules in subfolders and for some paths, memory found per repository and cut at 25 KB, imports four hops deep. It no longer hangs on a long AGENTS.md for Codex, and every part of its bar shows at a million tokens.
- A compaction summary showed as your prompt, and the meter stayed red with the context from before it.
- On the Task page every repeated prompt such as "continue" showed the first one's context.
- Undo Turn says when it also takes another agent's later turns.
- One torn line of the tasks log hid every task.
- An older `cellix` on the PATH rolled the Claude Code plugin back every session.
- Code blocks close only on a fence as long as the one that opened them, so output holding its own fence stays inside it on a transcript page.
- A paste on Windows that arrived in two reads could take a word suggestion.

### Changed
- When a narrow side pane's tabs don't fit, the tab in front shows whole, with a ⌄ that lists every tab and starts a new agent or terminal. The icons at the top right sit closer, on their row's own background.
- Section headings look one way in Source Control, the Timeline, Agent Context and the Task page.
- Show Source, Compare, Clear Finished, Start an Agent and Install are buttons. Merging is a label, with Abort beside it.
- Source Control's commit box stays in place while its lists scroll.
- A pane under the file hides with a bottom-panel icon.
- Palettes, signature help and other floating panels end on whole rows; the Task page's prompt box fills its width.
- Wording: "1 line", and a comparison no longer ends by asking for a commit.

## [0.6.0] - 2026-11-07

Cellix's goal now reaches past reviewing agents' work: it is the editor you keep open all day in the
terminal, with VS Code's everyday features built in, no extensions, and the work with agents at its
centre.

### Added
- Ask Agent on terminal output: select output in a side-pane terminal and the agent gets all of it, the command that printed it and every file of the project it names. Inside Claude Code it goes into Claude's prompt.
- Ask Agent on a Problems row, with what the checker said and the lines around it.
- An Inbox for what needs you, behind a bell in the status bar: an agent asking, a plan to review, a finished turn, a reply to a comment, a failed test run, across every workspace. While Cellix is behind another window, each one is also a terminal notification (`notify: false` turns that off).
- An agent's transcript as a page you can read and comment on, from Open Transcript on its tab. The tab also shows how full its context is, orange from 80% and red from 95%.
- An Agent Context panel on the rail: how the agent's context fills up, its instruction files in load order with their imports, its skills, subagents, hooks and MCP servers, the files it read, and checks for instructions that contradict each other, broken imports and cut-off memory. For Claude Code, Codex and Gemini CLI.
- Show Agent Task: one piece of agent work as a timeline of plan versions and turns, each with its prompt, changes, tests and comments, with Review Turn, Undo Turn, and Approve, Send Back and Compare for a waiting plan.
- Code folding (Ctrl+Shift+[ and ], Fold All, Unfold All), sticky scroll, word wrap (Alt+Z) and bracket pair colours.
- Go to Symbol in the file (Ctrl+Shift+O) and in the project (Ctrl+T), signature help and inlay hints.
- Run Task for package.json scripts, Cargo, make, just, go, pytest and `.vscode/tasks.json`, with the errors it prints in Problems. Ctrl+Shift+B runs the build task.
- Format on save, auto save after a delay or on leaving a file, and `.editorconfig`.
- A Keyboard page in Settings to see and change every command's keys.
- Open Recent, in the palette and on the welcome page.
- Merge conflicts settled in the file, with Current, Incoming, Both and Compare, then Mark Resolved and the merge committed or the rebase continued.
- Stash, Apply, Pop and Drop in Source Control; a commit graph at its end; the open file's commits in the Explorer's Timeline; Compare With the clipboard, a branch or a commit.

### Changed
- Docked beside Claude Code's chat, under 80 columns, the pane has its own layout: the panel icons lead its header, tabs that don't fit fold into "N more" chips, and the page menu folds into Format and Insert.
- The pane's close button is now a hide-pane icon, so the only × in the top right corner quits.
- The light theme is Catppuccin Latte.
- Menus and tooltips sit on whole rows, every sidebar has the same title row, buttons are one chip inside their row, and a pane's tabs are chips.
- "Needs you" means an agent is really waiting: Claude Code's idle reminder no longer counts, answering a prompt clears it, and an agent that was killed stops counting.

### Fixed
- Cancel in "Stop what's running?" missed clicks near its edges. Every button now takes a click anywhere it's painted.

## [0.5.5] - 2026-11-06

### Fixed
- Copying from an agent in the side pane did nothing. Claude Code takes the mouse and copies its own selection through the terminal's clipboard, which the pane dropped; it now reaches the real clipboard, and Ctrl+V in Cellix.
- A workspace with nothing running in it, only its files, didn't come back when Cellix was opened again, and `cellix ls` didn't list it. Every workspace open when Cellix quits now comes back, listed by `cellix ls` as "open, nothing running", until its tile is closed.

## [0.5.4] - 2026-11-06

### Added
- Resume All on the card of a session that stopped, when more than one did, so they all come back with one press rather than tab by tab.
- The Comments pane can show resolved comments, marked as resolved, and Reopen brings one back.

### Changed
- The close button at the top right is drawn as the icons beside it are, grey with the same hover chip, instead of a red circle. A thin line still sets it apart from the side pane's own ×.
- A numbered list on a page shows its numbers as Markdown counts them, and taking an item out or putting one in renumbers the rest in the file, undone in one step with the edit.

### Fixed
- Typing a comment works as in any text box: a space moves the caret, the arrows and Option+arrows move it, and Option+Backspace, Ctrl+W and Cmd+Backspace delete a word or the line.
- A resolved comment kept its words highlighted on the page until the file was next edited.
- Ask Agent inside Claude Code put only the first line of the selection in the prompt, cut at 200 letters. It now puts all of it. A comment sent to an agent also says all of what was selected.

## [0.5.3] - 2026-11-05

### Added
- A red close button in the top right corner quits Cellix, as Ctrl+Q does, asking first about unsaved files and what's running. It's a window's red circle, so it isn't mistaken for the side pane's own ×.
- Gitignored files and folders are dimmed in the explorer, as in VS Code. They're still listed, since agents often work in them, and what's dimmed is exactly what Quick Open and project search leave out.

### Fixed
- In Ghostty, only the top half of a workspace tile in the bottom bar took a click. Ghostty draws the last row down into its bottom padding, and a click there counted as below the screen; it now lands on the bar.
- A double click on a folder in the explorer opened and then closed it again. The second press of a double click now leaves it open, as in VS Code.

## [0.5.2] - 2026-11-05

### Added
- New Workspace finds the folder for you. It opens with suggestions, other projects running on the machine, folders beside this one and folders in your home, and typing narrows them. Type a path (`~/src/`, `../`, `/srv/`) to list the folders inside it, Tab to go into the picked one, and Enter to open it.

## [0.5.1] - 2026-11-05

### Changed
- The workspace bar is always there, even with one folder open, and its button says what it does: **+ New Workspace**, with "Open another folder beside this one" on hover. The small + it replaces was easy to miss, and with one folder there was no bar to find it in.

## [0.5.0] - 2026-11-05

### Added
- Cellix keeps your work over SSH without tmux. Agents and shells in the side pane already outlived a dropped connection; now the next `cellix` in a folder also has your open tabs and unsaved edits as you left them, and a bar along the bottom holds every folder with work running on the machine, each tile saying whether its agent is working, needs you, or is done. Click a tile to switch, + to open another folder, Ctrl+Alt+PageUp and PageDown to move between them. `cellix` in your home folder lists what's running to choose from, and `cellix ls` prints it.
- After the machine restarts, what was running comes back as it was left, with Resume for Claude Code and Codex, which picks up the agent's own conversation, and Start Again for terminals. Resume in the bar does every workspace at once.
- Two computers can show the same agent at once. Whichever typed into it last sets its size, as in tmux.
- On a Linux server that ends your programs when you log out, Cellix says so the first time something runs there and offers to turn on lingering, which keeps them.

### Fixed
- When the terminal turned dark (or light) while Cellix was running, as a Mac does at sunset, Cellix kept its old colours, and inside tmux even a new Cellix could start light: tmux answered from what it knew when you attached. Cellix now asks the terminal again whenever its window comes back into focus, and inside tmux lets the terminal tell tmux first.

## [0.4.3] - 2026-11-05

### Changed
- Inside Claude Code over SSH and in tmux, the pane is drawn as on your own machine, in pictures, instead of text. It goes a row at a time, and only rows the terminal hasn't shown cross: a click or a letter costs a few kilobytes, a scroll the lines that came into view. `cellix setup claude` adds `allow-passthrough` and `focus-events` to your tmux config, which Claude Code needs to show pictures inside tmux; in a tmux already running without them, Cellix turns them on and asks you to restart Claude Code once. Where pictures can't show, the pane is text, and says why.

## [0.4.2] - 2026-11-05

### Fixed
- In a big folder, Quick Open and the "Open a File" button held everything until the whole tree had been walked. Quick Open now opens at once and fills in as files are found, saying how many so far; typing stays quick over hundreds of thousands of files, and closing it stops the walk.
- In a repository with many untracked files, Cellix waited for `git status` before drawing anything, so the pane inside Claude Code sat on "Starting Cellix…". Git is now asked in the background.
- After an upgrade, the Claude Code plugin written by the older Cellix stayed in place, and over SSH it waited on "Starting Cellix…" for lines it couldn't read. Each session's start now brings the plugin up to date, and a pane started by an older plugin says to restart Claude Code.
- When Claude Code's PATH didn't include `cellix`, as when it's started from an IDE, a service or by its full path, the pane waited on "Starting Cellix…" for good and the hooks did nothing. Setup now records where Cellix is (`CELLIX_BIN`) for the plugin to use, and a pane that can't start says why.
- Inside Claude Code in tmux, Zellij, iTerm2 or any terminal without Kitty pictures, the pane was blank. It's now drawn as text cells there, and as a picture only in kitty and Ghostty.
- In a container or sandbox where Claude Code is process 1, the pane quit as soon as it started.
- Ctrl+W and a middle click close the Settings tab, as they do any other. Ctrl+W used to close the file hidden behind Settings instead, and a middle click did nothing.

## [0.4.1] - 2026-11-05

### Changed
- Inside Claude Code, Ask Agent puts the file, line and selected words into Claude Code's own prompt, not Cellix's comment bar, since the agent is already there. Press Esc to go back to the prompt and finish the question.

### Fixed
- Install buttons in Settings no longer hang when an installer asks a question on the terminal. terminal-browser's installer asked whether to add its agent skill and waited forever, so Settings said an install was still running after it had finished. Installs now run with no terminal and take the installer's default answer.
- Over SSH, the pane inside Claude Code no longer blinks on every mouse move, and clicks land at once. It is now sent as text cells, so only what changed crosses the connection; input goes in batches, a run of mouse moves keeps only the last, and Cellix draws once per burst of input. Pictures and PDFs show in their text form over SSH.

## [0.4.0] - 2026-11-04

### Added
- Tabs drag. A file's tab moves along the tab bar, splits a pane when dropped on its left or right half, and shows in another pane when dropped in its middle; the side pane's tabs move along its own row, and dragging one over the bottom half of the file docks the whole pane under it, until it's dragged to the right edge. A ghost of the tab follows the pointer, the place it would land is washed in the accent, the status bar says what dropping will do, and Esc cancels.
- Maximize. The button after the side pane's tabs, or a double-click on one of its tabs, lets the pane fill everything right of the activity bar; a double-click on a file's tab does the same for that file, hiding the sidebar, the other panes and the side pane. Both are in the palette as Toggle Maximized Side Pane and Toggle Maximized Editor. Opening something that a maximized part hides restores first.
- A page zoomed wider than the Preview can be dragged in any direction, and Shift+scroll or a sideways scroll moves it across. Thin scrollbars show where you are and can be dragged, and a hint says how the first time a page is too wide. A click without a drag still goes to the source, now on release. With the Preview clicked, Space types nothing, so Space+drag pans as in other viewers, and the arrow keys move the page.
- Windows, as a preview. Cellix builds for x86_64-pc-windows-msvc and runs through ConPTY on the cell tier, with PowerShell in the side pane. Its tests pass on a Windows CI runner, but it has not been run in a real Windows Terminal yet. On CI a click reaches Cellix through the ConPTY Windows Terminal ships, though not through Windows' older built-in console, so the mouse should work in Windows Terminal and may not elsewhere. `cellix comments`, `open` and the other commands reach it over a port on 127.0.0.1, where each end proves itself with a token kept in the user's local app data. Closing the window keeps unsent comments for next time, as quitting does, and a paste, which Windows delivers as keys, lands as pasted. The Codex and Gemini CLI hooks are written for PowerShell there, but they and the Claude Code mod haven't been tried on Windows. Releases carry `cellix-x86_64-pc-windows-msvc.zip`, which `cargo binstall` takes too. Not on Windows yet: the pane inside Claude Code, Kitty graphics and pictures from programs in the side pane, programs there that keep running after Cellix quits, and stopping a running notebook cell.
- A plan's comment threads keep their replies and resolved state in its next version. One whose words the new version dropped stays open with a "Removed in version N" pill, and clicking it shows what changed.
- A paper split over `\input` and `\include` files counts its words as one, puts Track Changes' package in the root's preamble, and rebuilds when an agent edits any of its open files.

### Changed
- The side pane's tab for PDFs and images is called Preview. With an image it says "image" where it said "PDF", its bar names the file, and a click on it no longer asks for SyncTeX.

### Fixed
- In a project with large untracked files, such as data, Claude Code's prompt hook could take longer than the 30 seconds Claude Code gives it ("UserPromptSubmit hook timed out"), and the comments it carries never reached the agent. Each prompt's checkpoint read every file again and copied untracked ones into `.git`. It now keeps its own index, so only changed files are read; untracked files over 10 MB stay out of checkpoints; and the hook waits at most 3 seconds, leaving a slow checkpoint to finish on its own.
- Inside Claude Code, the wheel scrolls the Comments pane when the pointer is over it. It used to scroll the file whatever was under the pointer.
- A comment on a plan no longer moves onto words that say the opposite in the next version: only the same words, rewrapped, count as found, and reworded ones keep their quote with a "Changed in version N" pill. An approved plan's threads are settled with it, and the next plan starts at version one instead of inheriting them.
- A section of a split paper belongs to the document that really takes it in, found through `% !TeX root` in any case and up to three folders above, so Track Changes no longer writes into a file that only mentions the section's name. Building saves the paper's other open files first, and Cite reads the bibliographies the paper names, `\addbibresource[...]` included.
- Warnings in a section file point at that file and count once, a failed export keeps the last good PDF, pictures in a section file are found from the paper's root and its `\graphicspath`, forward search tells apart chapters with the same file name, and the pdflatex fallback runs Biber for biblatex, reruns while the log asks, and says when BibTeX fails.
- The Linux install line works with GNU tar, which most Linux machines have: it extracts `./cellix`, makes `~/.local/bin` if needed, and stops on a failed download.
- Export to PDF works with TeX Live's pdflatex and latexmk, not only tectonic: the export no longer builds from a hidden file TeX Live refuses to write beside, and it loads TikZ, tikz-cd and ulem only when the document uses them. Build PDF and Export are now tested against TeX Live on CI.

## [0.3.1] - 2026-11-04

### Added
- natbib citations read as they print: `\citet` gives `Knuth (1984)`, notes give `[see Knuth 1984, p. 3]`, and `\citeauthor` and `\citeyear` give their part. Up to two options are read for any cite command.
- Callouts with a title of their own, such as `> [!tip]- Keep it short`, and Obsidian's callout kinds show their title on the page and keep it in HTML and PDF exports.
- One line of a change can be undone in Review: right-click a changed line for Remove This Added Line or Put This Line Back, beside Comment.

### Changed
- Track Changes is remembered for each document, in the project's cache folder.
- Saving a document with suggestions still open says they stay in the file as marks.
- Approving a Codex plan with changes or notes starts one more turn that hands them to Codex.
- Quick Fix's first item is selected, so Enter applies it, as in VS Code.
- In the Comments pane, suggestions have dashed edges, buttons are as wide as their words with the same room either side, and Check my review carries the agent's icon.

### Fixed
- Under tmux with `allow-passthrough on`, Cellix asks the terminal outside tmux for its real cell size, and keeps that size on a resize. A console that passes tmux no pixel size can still show a thin line under every row; see Over SSH in the README.
- Clicking the PDF lands on the line clicked, where it landed most of a line above the pointer.
- Author names cite whole: `{World Health Organization}`, `van der Berg, Jan` and `Jan {van der Berg}`.
- A comment on a sentence that appears twice stays on its own copy, and when its quote is reworded the card shows the new words.

## [0.3.0] - 2026-11-03

### Added
- Check my review, at the top of the Comments pane, asks the agent to give each of your comments a verdict and to add the comments a careful reviewer would. Verdicts show as "Agrees" or "Pushes back" under your comment, suggestions arrive as "Suggested by Claude" cards with Keep and Dismiss, and a strip counts comments by kind (Clarity, Correctness, Evidence, Structure, Reproducibility). Agents answer with `cellix reply` and the new `cellix suggest <file> <line> <kind> "<words>" "<comment>"`.
- `cellix reply <id> --suggest "<new words>"` answers a comment with a rewrite. The card offers Apply, which puts it in place of the quoted words as a suggestion, and Not this.
- Next Change and Previous Change (Alt+F5, Shift+Alt+F5, and the palette) walk a document's suggestions and say which of how many you're on.
- Quick Fix… (Ctrl+., the palette, or the code's right-click menu) lists the language server's code actions at the caret and applies the one you pick, across files if needed.
- A LaTeX build's warnings appear in Problems, including undefined references and citations at their line, so F8 walks them.
- Code comments no agent has taken yet survive a restart. The next Cellix in that folder takes them back and says how many are waiting.

### Changed
- Under Track Changes, every agent edit arrives as suggestions, even after you've accepted or rejected some, and Replace and Replace All write suggestions too. Only your own typing falls back to "changed on disk".
- Approve on a plan sends your comments with it, as a "Reviewer notes" section, so a caveat reaches the agent before it starts the work.
- A paper's references list numbers as the PDF does: sorted by author for the plain styles, in citing order for unsrt, with "[?]" for a key the bibliography lacks. Two authors cite as "Erdős and Rényi 1959", and three or more as "et al.".
- The Comments pane follows its design: each card has a header such as "You · Correctness", agent answers sit in an inset block with their verdict on a chip, Check my review is the pane's one filled button, and the coverage strip is pills. The rail's icons are larger and spaced a row and a half apart, two rows on the cell tier.

### Fixed
- Under tmux with `extended-keys on`, Ctrl keys such as Ctrl+Q and Ctrl+S reach Cellix again, and Ctrl+Shift chords no longer arrive unshifted. Cellix asks tmux for extended keys only when `extended-keys-format` is `csi-u`, which the README's tmux setup now sets. The graphics probe no longer shows up as tmux's window title, and Cellix reads tmux's `client_termtype`, so a stand-in TERM such as `vt220` over a console no longer drops it to text cells.
- Closing the terminal, or a SIGHUP or SIGTERM, ends Cellix the way Quit does: the workspace socket, IDE lock and held plan folders are cleaned up, and `--wait` comments are written. A stop that lands as the terminal closes no longer leaves Cellix running at full CPU.
- Comments on a plan held through an agent hook now go with Send back, where the bar said "No comments".
- Neovim mode: `:q!`, `:wq!`, `:x!`, `:qa!`, `ZZ`, `ZQ`, `:bd` and `:bw` no longer end Neovim or lock the buffer, and `:w!`, `:up` and `:wa` save. Each goes through Cellix to save, close the tab or quit.
- LaTeX: `\input`, `\include`, `\bibliography`, `\maketitle` and the like on the line after text start their own block, where they showed as raw source and broke numbering. Strike adds `\usepackage[normalem]{ulem}` so the paper builds, and turning on Track Changes in a saved paper saves the `\usepackage{changes}` it adds.
- Ctrl+R with Find open shows the replace row. Accept All and export leave marks inside inline code as code, and a reviewer's `{>>note<<}` stays out of HTML and PDF exports.

## [0.2.2] - 2026-11-03

### Added
- Go Back (Ctrl+-) and Go Forward (Ctrl+Shift+-), also in the palette, return to where Go to Definition, Go to Line or a search result jumped from.
- Hover: rest the pointer on code and its language server's type, signature and docs show under it. Show Hover in the palette asks about the symbol at the caret.
- VS Code's everyday keys: Ctrl+Shift+K deletes the line, Ctrl+Enter and Ctrl+Shift+Enter open a line below or above, Ctrl+Shift+L selects every occurrence, Ctrl+Alt+Up and Down add a cursor, F8 and Shift+F8 walk a file's problems, Ctrl+N opens a new document, Ctrl+, opens Settings, Ctrl+] and Ctrl+[ indent and outdent, and Ctrl+Tab switches tabs. Several need the Kitty keyboard protocol; elsewhere they're in the palette. Settings' Ghostty card now frees Cmd+A as well as Cmd+D.
- `\bibliography` shows a References list on the page with every cited entry, and a key the bibliography lacks says so in red.
- The Outline lists code's symbols from its language server, nested under their parent.
- Sections in `\input` files count toward the paper's numbering, and the Outline lists their headings in place; a click opens that file.

### Changed
- Reject on a hunk in Review, or on a proposed change, asks "Why reject it?". Enter rejects and Esc keeps it, and a reason goes to the agent as a comment on the changed line, so it doesn't try the same again.
- Every comment carries its quote, the selected words or the line, in code, documents and plans alike. A comment on a removed line says so. Comments reach the agent in file and line order as `file line N, on "quote": text`, and `cellix comments` gives `quote` and `removed`.
- Compare Plan Versions diffs by line: removed lines are struck, new lines marked, and a line changed in place shows its words. A plan's comments carry into its next version as threads that follow their quote.
- A paper an agent rewrites rebuilds its open PDF, as saving does.

### Fixed
- Neovim mode: `:e`, `:b`, `:split` or `:tabnew` could point Neovim at another file while Cellix showed this one, so `:w` wrote the wrong lines. Such commands are now undone, and the status says to open files with Ctrl+P and split with Ctrl+\.
- With Track Changes on, a second agent edit in the same turn no longer meets "changed on disk": while its suggestions stand untouched, a turn's edits pile up into one review.
- A paper finds the `.bib` files it names with `\bibliography` or `\addbibresource` wherever they are, so a bibliography in another folder no longer leaves every citation red and completion empty. TeX accents in a `.bib` read as letters.
- Closing `\Cref{…}` at the end of a line no longer moves the caret to the line's start.
- A suggestion that runs across paragraphs shows as one, with its markers hidden. `\bibliographystyle` is hidden instead of running into the next line as source.

## [0.2.1] - 2026-11-03

### Fixed
- Scrolling on the cell tier, as under tmux over SSH, no longer rewrites every row of the editor on each wheel step. Cellix has the terminal shift the rows with a scroll region and sends only the lines coming into view, less than half the data of a redraw.

## [0.2.0] - 2026-11-03

### Added
- With Track Changes on, a file an agent rewrites arrives as suggestions, word by word, in CriticMarkup or the `changes` package, to accept or reject before saving.
- Plan versions: each plan an agent sends again keeps the one before it as `Plan v1.md` and on, comments follow their text into the new version, and Compare Plan Versions shows what changed while the plan waits.
- Find and Search have Match Case, Match Whole Word and Use Regular Expression, as buttons in the field and as Alt+C, Alt+W and Alt+R. Find gains a replace row on Ctrl+R: Enter replaces and moves on, Alt+Enter or All replaces every match as one edit, and `$1` takes a regular expression's group. Project search lists every match on a line.
- Typing inside `\cite{` or `\ref{` (and `\citep`, `\eqref`, `\cref` and the rest) offers the bibliography's keys or the paper's labels, and Enter or Tab puts the whole key in. A reference to no label or a citation the bibliography lacks shows in the error colour.
- A paper has its own right-click menu, with Typewriter, Paragraph, Section, Verbatim, Insert Citation, Build PDF and Show in PDF, and the file tree hides the files a build writes beside it (`.aux`, `.log`, `.synctex.gz`, `.fls` and the rest). latexmk and pdflatex builds write SyncTeX, and pdflatex runs BibTeX and builds again until references settle.
- When `terminal-browser` isn't installed, the corner card offers to install it, as does a Browser card under Settings › Integrations.

### Changed
- The `vim` keymap runs the real Neovim when it's installed.
- The status bar's word count counts prose only: no Markdown markup, code, link targets, LaTeX commands, keys or math. A selection shows as "12 of 340 words".
- In Claude Code, `/cellix-send` and the pane's Send to Agent start a turn with your comments straight away, where they waited for your next prompt. `/cellix <file>` opens the file, where the argument was ignored, and a second session in a folder that already has a Cellix says so instead of waiting on "Starting Cellix…".
- A comment follows its quote when the text is rewrapped or a word or two changes, and the Comment and Ask Agent bar sits above the first selected line.
- Export sets front matter as the title, author and date in HTML and PDF, and a document with pending suggestions asks whether the PDF shows them accepted or as they stand. `==` inside inline code stays code, and an existing `name.tex` no longer stops a Markdown file's PDF export.
- One icon set across the app, a narrower rail with line icons, a Find bar on a raised panel with its switches inside the field, an accent on the tab in front, and one colour for matches in the file and the Search panel.

### Fixed
- The built-in Vim mode no longer corrupts files. It knows `iw` and `aw`, `w` and `e`, `f`/`t`/`F`/`T`, `V`, `D`, `C`, `J`, `r`, `/` and the `:` commands that save, close and go to a line, and swallows anything else instead of typing it.
- The plan banner names the agent whose plan is waiting, where it always said Claude Code.
- A brace still being typed in LaTeX shows on the page, Ctrl+\ splits the editor in terminals that send it as Ctrl+4, and a table's caption in a paper stays at the left with its table.

## [0.1.0] - 2026-11-03

The first public release. Cellix is a terminal editor that lives inside your AI coding agent, for reading and correcting what it writes: plans, diffs, code, Markdown and LaTeX. Binaries are on the releases page for macOS and Linux on both architectures, with a Homebrew formula in `quill-27f/tap`.

### Added
- A Claude Code mod, a Codex plugin and a Gemini CLI extension, each set up with `cellix setup claude`, `cellix setup codex` or `cellix setup gemini`. Inside Claude Code, Cellix opens as a pane beside the conversation with `/cellix`, and by itself when Claude proposes an edit or a plan.
- Plans open as a page with Approve and Send back, and your edits and comments go with the answer.
- Review (Ctrl+G) stacks changes hunk by hunk against HEAD or any agent turn, with the tests the agent ran. Accept stages a hunk, and `Undo Agent Turn` puts the tree back.
- Comment or Ask Agent on any selection, picture, diagram, formula or PDF. Comments are threads in the side pane, reach the agent with their file, line and quote, and the agent answers with `cellix reply`. `cellix plan`, `cellix comments --take` and `cellix reply` work for any agent.
- Markdown and LaTeX open as one editable page over a plain-text file: math typeset in pure Rust, Mermaid, Graphviz and SVG diagrams, TikZ built in the background, tables, images, footnotes, callouts, Track Changes, and offline spelling and grammar.
- LaTeX papers with sections, citations and figures across `\input` files, Build PDF (Ctrl+Alt+B) with `tectonic`, `latexmk` or `pdflatex`, the PDF in the side pane, and SyncTeX both ways. Markdown exports to PDF or a web page.
- A side pane that runs a real agent, a terminal or a browser beside your file, from the icons at the top right. Ctrl+J brings up a terminal, and what runs there can outlive Cellix.
- Source Control in the sidebar stages, commits, pushes and pulls, as in VS Code.
- A full editor on its own: highlighting in every file type, suggestions with no setup, language servers with one-click installs in Settings, Go to Definition, rename, references and formatting, project search and replace, a minimap, and Vim and Neovim keymaps.
- CSV and TSV as editable tables with Excel's keys, and Jupyter notebooks run on a real kernel.
- Real pixels in terminals with Kitty graphics, such as Ghostty, Kitty and WezTerm, with fonts built in, and text cells anywhere else. It runs under tmux, Zellij and over SSH, with light and dark themes that can follow the terminal.

[0.5.5]: https://github.com/quill-27f/cellix/compare/v0.5.4...v0.5.5
[0.5.4]: https://github.com/quill-27f/cellix/compare/v0.5.3...v0.5.4
[0.5.3]: https://github.com/quill-27f/cellix/compare/v0.5.2...v0.5.3
[0.5.2]: https://github.com/quill-27f/cellix/compare/v0.5.1...v0.5.2
[0.5.1]: https://github.com/quill-27f/cellix/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/quill-27f/cellix/compare/v0.4.3...v0.5.0
[0.4.3]: https://github.com/quill-27f/cellix/compare/v0.4.2...v0.4.3
[0.4.2]: https://github.com/quill-27f/cellix/compare/v0.4.1...v0.4.2
[0.4.1]: https://github.com/quill-27f/cellix/compare/v0.4.0...v0.4.1
[0.4.0]: https://github.com/quill-27f/cellix/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/quill-27f/cellix/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/quill-27f/cellix/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/quill-27f/cellix/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/quill-27f/cellix/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/quill-27f/cellix/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/quill-27f/cellix/releases/tag/v0.1.0
