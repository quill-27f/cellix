# Changelog

All notable changes to Cellix. Versions follow semver; each one is on the [releases page](https://github.com/quill-27f/cellix/releases).

## [Unreleased]

### Added
- Windows, as a preview. Cellix builds for x86_64-pc-windows-msvc and runs through ConPTY on the cell tier, with PowerShell in the side pane. Its tests pass on a Windows CI runner, but it has not been run in a real Windows Terminal yet. On CI a click reaches Cellix through the ConPTY Windows Terminal ships, though not through Windows' older built-in console, so the mouse should work in Windows Terminal and may not elsewhere. `cellix comments`, `open` and the other commands reach it over a port on 127.0.0.1, where each end proves itself with a token kept in the user's local app data. Closing the window keeps unsent comments for next time, as quitting does, and a paste, which Windows delivers as keys, lands as pasted. The Codex and Gemini CLI hooks are written for PowerShell there, but they and the Claude Code mod haven't been tried on Windows. Releases carry `cellix-x86_64-pc-windows-msvc.zip`, which `cargo binstall` takes too. Not on Windows yet: the pane inside Claude Code, Kitty graphics and pictures from programs in the side pane, programs there that keep running after Cellix quits, and stopping a running notebook cell.
- A plan's comment threads keep their replies and resolved state in its next version. One whose words the new version dropped stays open with a "Removed in version N" pill, and clicking it shows what changed.
- A paper split over `\input` and `\include` files counts its words as one, puts Track Changes' package in the root's preamble, and rebuilds when an agent edits any of its open files.

### Fixed
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

[0.3.1]: https://github.com/quill-27f/cellix/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/quill-27f/cellix/compare/v0.2.2...v0.3.0
[0.2.2]: https://github.com/quill-27f/cellix/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/quill-27f/cellix/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/quill-27f/cellix/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/quill-27f/cellix/releases/tag/v0.1.0
