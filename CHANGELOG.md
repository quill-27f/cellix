# Changelog

All notable changes to Cellix. Versions follow semver; each one is on the [releases page](https://github.com/quill-27f/cellix/releases).

## [0.13.1] - 2026-11-22

The keyboard starts in the agent, and Claude Code's cursor shows only where your typing goes.

### Changed
- A workspace that opens with an agent in front of its side pane starts with the keyboard in that agent, where it started in the file, so the first thing typed reaches it. Not where a file was named on the command line, or a dialog or Settings is open; a workspace you come back to keeps the keyboard where you left it.

### Fixed
- Claude Code's cursor showed in the side pane while the keyboard was elsewhere, so it looked as if typing would go there. A program in the side pane that asks to be told (focus reports) is now told when it gets the keyboard and when it loses it: a click elsewhere, another tab in front, another workspace, or Cellix's window behind another. Claude Code draws its cursor only while it has it. A session kept running while Cellix was closed is told again when Cellix comes back, and the file's caret hides while the pane has the keyboard.
- A helper shown live over the pane (peek) takes the keys while it's open; they went to the tab under it.

## [0.13.0] - 2026-11-22

Kiro CLI joins the agents, and a folder a click opened stays open.

### Added
- Kiro CLI, as an agent in the side pane: `cellix setup kiro` (or Install under Settings › Integrations) writes one file of Cellix's own, `~/.kiro/hooks/cellix.json`, among Kiro CLI's global hooks, and `--remove` takes back exactly that file; your Kiro agent files are left as they are. Kiro says when it's working, when it asks to run something (needs you, with what it asks) and when it's done, on its tab, in the Agents list, on the workspace's tile and in the Inbox, and its tab is named by what you first asked it. The + menu and START offer it where `kiro-cli` is installed, it has a mark of Cellix's own, Resume starts its session again with `kiro-cli chat --resume-id`, and `kiro-cli` typed in a Terminal tab makes that tab Kiro's. Its hooks need Kiro CLI 3.0 or later, which setup says on an older one. It takes your comments through Ask Agent; Ask isn't offered for it yet. It hasn't been tried with the real Kiro CLI.

### Fixed
- A click that opened a folder in the Explorer sometimes shut it again a moment later, as if clicked twice: a look at the disk that had started before the click came in with the tree as it was. Such a look now leaves the tree alone and only brings git's marks; the next one shows the tree as it is.

## [0.12.1] - 2026-11-20

The cursor blinks, every box edits text as the others do, and an agent's tab is named by the conversation it holds, wherever it runs.

### Added
- The cursor blinks, as VS Code's does, half a second each way: in files and pages, in every text box, and in a terminal pane unless its program asks for a steady cursor. Any key, click or move shows it solid and starts the blink again, and it stays still while Cellix's window is behind another. Settings › Editor › Cursor blinking turns it off (`cursor_blink` in `~/.config/cellix/settings.json`). In the cell tier the terminal blinks its own cursor; elsewhere a blink re-shows a picture the terminal already holds, a few hundred bytes a second, and a window in the background wakes for nothing.

### Changed
- An agent's tab is named by the conversation it holds, wherever it runs. Cellix reads Claude Code's own record of each running session (`~/.claude/sessions/<pid>.json`, under `CLAUDE_CONFIG_DIR` when set) for its conversation, and names the tab by that conversation's title or its first prompt, so a tab no longer depends on which `cellix` the plugin's hooks find first on the PATH. After `/clear`, `/resume` or a new conversation in the same tab, the name, the first prompt and the context meter start over.
- `claude` typed at a Terminal tab's prompt names and marks that tab as an agent's tab, by its conversation, where it read "claude"; its spinner and dots follow its hooks, and when it exits the tab is a terminal again.
- Every text box edits as the comment box does: Rename tab and every other prompt (Rename Symbol, a file's name, a stash's message…), both of the Find bar's fields, the Search sidebar's boxes, Settings' filter, the commit message and New Worktree. They have a caret that the arrows, Home and End move, a word at a time with Alt or Ctrl, Option+Backspace and Ctrl+W delete the word before it, Delete, Cmd+Backspace and Ctrl+K clear to the line's ends, paste goes in at the caret, and a click puts the caret where you press. Where they took letters and Backspace only, Ctrl+W in a box now deletes a word rather than closing the tab behind it.
- A box's caret stands where it is, between two letters and clear of both, and a text longer than its box shows the part the caret is in.

### Fixed
- Pasting while the Search sidebar or Settings had the keyboard went into the file behind them, and the emoji picker's field didn't take the word and line deletes.
- Claude Code tabs stayed "Claude Code" on a machine where the hooks' `cellix` was an older one, or where Claude Code was started from a Terminal tab.

## [0.12.0] - 2026-11-20

Ask: select words on a page, in code or in a terminal's output and ask the agent about them, as a side question that comes back on a card, not as a comment. Agents' tabs are named by what they're working on, every comment shows its number, and "#33" goes to comment 33, whose words you can now select and copy. Installing a language server works on Linux without root, and Settings says how each one installs before you press. The Agents list shows only agents you can act on here, the comment order you pick is kept for every Cellix on the machine, and a round of polish evens out tabs, buttons, cards and pages.

### Added
- **Ask.** A selection's buttons are Comment and Ask, on a page, in code and on a terminal's output, and Alt+A opens Ask: a card under the words with a field, and Explain, Define and Simplify for the usual questions. The agent's own command line answers in the background, read-only, with the whole document (the selection marked, the headings over it, a paper's `\input` files read in), so a question about one word is answered from everything around it. The answer streams onto the card, with Copy, Follow Up (which goes on in the same side conversation), Make a Comment (for when it was feedback after all) and Dismiss. Every later question about the same document goes on in its one side conversation. Where Claude Code works here, "With the agent's conversation" asks in a copy of what it already knows, and leaves its own conversation as it was. An answer is never a comment: it's never counted, sent or handed to the agent. Claude Code, Codex and Gemini CLI can answer; with none installed the card says what Ask needs.
- The order you pick in the Comments pane (Document order, Newest first, Oldest first) is kept in `~/.config/cellix/settings.json` as `comment_sort`, so every Cellix on the machine opens with it; one already running follows the next time you open the menu, show the comments or come back to the window. A workspace's `.cellix/settings.json` can set its own.
- Each language server in Settings › Editor says how it installs here, beside its name ("npm", "brew", "GitHub"), and the whole command when pointed at. The card that offers a server says it too, with the command set as code, and what Copy Command copies.
- "Set up more agents…" at the end of Review with… opens Settings at Integrations.
- A workspace's tile has the needs-you dot while its Inbox holds a failed test run or an agent's answer to a comment.
- Each comment's card ends its head with the comment's number ("You · Clarity · #1"), as do the glance card and the review's Markdown, and "#33", "comment 33" or "Comment #33" in a comment, a reply or a terminal's output is a link: a press opens that comment as its ◆ does (a resolved one is shown, and a missing one is said), a right click has Go to Comment #33, and in a terminal it's a Ctrl+click.
- A comment card's words can be selected: drag, double-click a word, triple-click a line or Shift+click, and Ctrl+C copies them, line breaks kept. A right click on a card has Copy, Copy Comment and Copy Quote, with Open Link and Copy Link over them on a link.
- The graph in Source Control stands each branch and tag as a chip before its commit's message, in its lane's colour, as VS Code's does; a long name is cut in its middle.

### Changed
- An agent's tab is named by what it's working on: the name it gives itself, else its conversation's title, else what you first asked it, cut at a word's end, where every Claude Code tab used to read "Claude Code". Tabs that still read alike are told apart by "(2)", the tab's tip has the whole name, and Rename still wins. The Agents list names them the same way.
- Ask Agent on one comment's card sends that comment alone. The agent used to be handed every other comment that waited too, and answered them all; they now wait for your next prompt or Send.
- Install for a language server runs the first way this machine has: Homebrew where it's on the PATH, npm into `~/.local` where npm's own folder isn't yours, the project's release build fetched into `~/.local/bin` (static musl builds on Linux where the project makes them), or rustup, `go install`, `dotnet tool` or `gem --user-install`. A server only a system package gives says what it needs ("Needs … or sudo dnf install …") with Copy Command, and opening the file again looks for the server once more. Servers are found and started from `~/.local/bin`, `~/.cargo/bin`, Go's bin and `~/.dotnet/tools` even when the PATH lacks them.
- The Agents list shows only agents with a tab here, and their helpers: every row does something when pressed. Agents running in other terminals no longer show there, in counts, on the bell or on the workspace tile; they still hear about your comments.
- Tabs behind share a short row evenly: each keeps the same number of letters, three at the least, and they go to their marks only together. A first tab cut to its mark gets a little more room, and the + stands in the middle between the last tab and ⤢. Where the row is shorter still, the name in front is cut, keeping its "(2)", before ⤢ gives way, and only then do tabs go into the chevron's list.
- A button's words are centred on a capital's height, so every label of a size has one baseline, and a button in a row of words stands on their line. Every floating card keeps 24 px inside its edge on all four sides, and the cards that ask you something put their buttons at the right, the answer last.
- A line's number takes its worst problem's colour, warnings in amber, and the word where the problem starts is underlined in it, from a task's problems as well as a language server's.
- Review with… lists only agents that are installed. An idle agent's name reads as plain text, not greyed out, and a helper's name is lighter than its agent's.
- Check My Review is a quiet button, so only one button in view is filled.
- Settings' language servers stand in one column: every tag ends at one edge, a long name is cut before it, and a pointed-at row stays quiet.
- The workspaces share the status bar's row only where it still holds the status bar's words at their fullest; otherwise they have a row of their own, where New Workspace keeps its words.
- In the light theme the needs-you chip and a transcript's "failed" read at 4.5 to 1.
- A tip keeps 8 px between its words and its edge, over and under.

### Fixed
- A terminal's late answer to a question Cellix had stopped waiting for was typed into the open file as keys (`11;rgb:1010/1111/1414`). It's dropped wherever it arrives, whole or in pieces.
- Installing a language server from Settings failed on Linux: most were Homebrew installs, npm wrote to a folder only root could, and there was no terminal for sudo. A failed install now says why on its row and on the card, and both stay until you've dealt with it.
- "Spaces: 4" no longer drops off the status bar while a language server says it's fetching.
- Sticky scroll keeps a scope's header while one of its lines is the first under the headers, and Go to Line, search results, problems and definitions put their line under the pinned headers instead of behind them.
- A LaTeX drawing that never answers is stopped after a minute and the page says so, instead of "Drawing with LaTeX…" for ever; with no LaTeX installed it says that.
- A selection's buttons stand 12 px from it, beside its start, and never past what follows its paragraph, or under it where a label such as a paper's Abstract stands over it; the same for selected terminal output, ending where it ends.
- An insertion's underline stops at its last letter; a drawing's note, a picture under open source and the scrollbar beside Show Source keep their air; the emoji picker is as tall as what it finds; a pie's slices are their swatches' colours and stand out from the page in both themes; a table cut at the page's edge shows no part of a cell's words; an equation's number stands on its line; a misspelt word in a table's cell keeps its dots off the cell's line; a wide code box's scrollbar stands inside it, clear of its code.
- A reviewer's note's tint is cut flush where it wraps; Abort stands on the line of the words beside it; a submenu stands against its menu without covering its edge.
- The state dot on an agent's tab stays put when the tab comes forward, chips in a tab row are one height, a tip hides the terminal cursor under it, and a dragged tab's bar stands clear of the pane's line.
- A frame drawn after news from an agent could put the workspaces' row over the status bar: the window is fitted when news comes in, not only on a key.
- A state diagram's labelled edge to the state under it runs straight down with its label beside it, where its arrowhead stood on the label.
- A card over a picture or a drawing hides only what it covers, where the rows at its top and foot were cleared whole and cut an equation under it in half.
- A pixel of the terminal's prompt no longer shows at a tip's rounded corner, and the program in the side pane starts 8 px under the line over it.

## [0.11.0] - 2026-11-19

Agents among themselves: they can start, message and wait for each other, and you see at a glance which one is working, which finished and which needs you. Reviewing a document goes further: your comments reach the agent from a page as from code, a card shows what changed since you wrote it, an agent's suggestion is accepted where it stands, and the review comes out as Markdown. Shift+click selects across a whole document, a link can be copied, and Cellix is calmer: idle, it draws and sends nothing. The dark theme is herdr's graphite.

### Added
- `cellix agent run`, `send`, `read`, `wait`, `ls` and `stop`: an agent can start another in the side pane with a task, message it, read what it last said, and wait until it stops working. A message waits while you're halfway through typing in that agent, and is never typed into an agent that's asking you something. Between agents a message arrives marked with who sent it. `cellix agent --skill` prints how agents use them, and the Claude Code, Codex and Gemini plugins carry it as a skill.
- The first time an agent starts another in a folder, Cellix asks you: Allow, Always Allow Here, or Deny. While it asks, that agent's tab and row say it needs you.
- An agent started by another is its helper. The agent that started it hears when it needs you, fails or ends, can wait for all of its helpers, and takes them with it when it ends. A helper has no tab of its own: it stands under its agent in the Agents list with its own mark (a helper asking you something brings its agent up to Needs you), and a click there opens it live over the pane (peek), with Keep as Tab.
- Ctrl+click an agent's name in a terminal's text to peek at it.
- A right click on a link, in a comment or a reply, on a page, in code or in a terminal's output, has Open Link and Copy Link. Copy Link takes the link's real address (a Markdown link's target, not its words) to your clipboard, over SSH too. A link to a file has Open File and Copy Path.
- Shift+click selects from your last click to here, on any page (Markdown, LaTeX, a notebook, a plan, a transcript, a CSV), in code and in a terminal's output, however far you scrolled in between. Another Shift+click moves the end. Cellix asks the terminal for Shift+click, which Ghostty and xterm otherwise keep for their own selection.
- Dragging a selection past the top or bottom of a pane scrolls it, faster the further past you hold, and turning the wheel while dragging scrolls with the selection following.
- A comment waiting for an agent says on its card how it gets there: with your next prompt to Claude Code, Codex or Gemini, or with Ask Agent. "Send N to Claude" in the Comments pane sends everything waiting in one message.
- An agent can say it addressed a comment, with `cellix reply <id> --done "…"`: the card says "Addressed by Claude" with Resolve and Not Done, and Resolve N addressed settles them together. You resolve, not the agent.
- When the words you commented on change, the card keeps what you quoted and shows, a word at a time, what changed since; if they're gone, it says so.
- Comments and replies show when they were made. A press on a kind's pill in the Comments pane shows only that kind; you pick a comment's kind from chips as you write it; and you can reply on an agent's suggestion without keeping it.
- Next Comment, Previous Comment and Check My Review are in the palette, and a press on a line's ◆ in code opens its comment. In a narrow pane cards stay short until one is lit.
- A tracked change on a page (Markdown's CriticMarkup, LaTeX's `changes` package) has Accept and Reject chips beside it when the caret or the pointer is on it. The Comments pane counts the suggestions and has Accept All, Reject All and Next.
- Accept Rewrite takes an agent's rewrite in one step over every line you commented on, as one edit you can undo.
- Comments: Copy Review as Markdown and Save Review as Markdown… (in the palette, and on a right click on the Comments tab) write the whole review out to share: each file's comments by kind, with their quotes, your words and every reply. Saving writes `review.md` into the project and opens it.
- With the Comments pane shut, resting the pointer on commented words on a page shows the comment on a card under them, with its last reply; a press on the card opens it in Comments.
- Review with… (in Review, and the palette's Agent: Review Changes with…) starts an agent on your changes as a reviewer. What it finds shows under the lines it's about, with Keep and Dismiss; a kept one becomes your comment.
- + after the side pane's tabs drops a menu: Terminal, each agent you have (keys 1–9), and roles. A role is a prompt file in `.cellix/roles` or `~/.config/cellix/roles`; Reviewer and Tester come built in.
- New Worktree… (on that menu and in the palette) makes a branch of its own for an agent, beside the project, opens it as a workspace and starts the agent in it. A worktree's row in the Agents list has Remove Worktree…, which never deletes the branch.
- A button at the end of the views hides the sidebar, and the same mark at the start of the tabs brings it back. Ctrl+B still does both.
- A spinner where something is working: on an agent's tab and its row in the Agents list, on a shell's tab while a command runs, and where Cellix itself waits (a language server indexing, a search, a push or pull, an install). Settings › Appearance › Motion stills it, and inside Claude Code's pane, where each turn would cost a new picture, it stands still.
- Any program in the side pane can say how it is with one line, the open OSC 7501 status sequence (`printf '\e]7501;state=blocked:app=my-agent\e\\'`), and shows as an agent does: working, needs you, done.
- OpenCode, GitHub Copilot CLI and Cursor's agent CLI connect as Claude Code, Codex and Gemini do: `cellix setup opencode|copilot|cursor` (and `--remove`). Cursor's agent has no hook for asking you something, so it never shows as needing you.
- `CELLIX_PROF=1` writes a line a second of what Cellix's loop did, and `harness/x perf` measures it against the last release.

### Changed
- The dark theme is herdr's graphite: near-black greys, herdr's softer blue, and its colours for working, needs you and done. The frame still takes your terminal's background, so on a terminal whose background isn't #101114 every surface keeps graphite's steps from that colour instead. The light theme is unchanged.
- An agent's tab has its mark, its own name (from the agent's title, or the one you give it with Rename), its context, and what it's doing: a spinner while it works, a dot when it needs you or finished while you were elsewhere. In a narrow pane a tab keeps its mark and its state.
- A shell's tab is named after the program running in it, with a spinner once it's run a second.
- The Agents list stands by what needs you: Needs you, Done, Working and Idle, each with how long since it changed.
- The marks for agents are Cellix's own: a burst for Claude Code, a hexagon for Codex, a star for Gemini, a prompt for a shell.
- The mouse wheel over a row of tabs that don't all fit moves it sideways.
- Idle, Cellix draws and sends nothing, and wakes only when something is due (twice a second, where it looked twenty times); it used to lay the window out and send a little twice a second, which over SSH woke tmux and your terminal each time. A program printing in a tab you can't see no longer redraws the window. Output on screen is drawn as it comes, at most once a frame, and what you type in the side pane is echoed at once: about a frame per key, where it was two or three.
- In Ghostty on the same machine, pictures go as files Ghostty reads, about a twentieth of the bytes. `CELLIX_TILE_FILES=0` turns it off.
- On a terminal without pictures, the cursor is hidden while a frame is written, so it doesn't run across the screen.
- The narrowest sidebar is 23 columns, to hold every view beside its button.
- In the dark theme, a picked row is tinted with the accent, as in the light one, and plain hovering stays grey; Find's other matches are stronger and the one you're on has a ring of its own.
- Every action that isn't the main one is one quiet button with a hover you can see, and a row of choices (in Settings, in a dialog) is one track.
- Every menu has the same half row over and under its rows, and hangs 8 px under the line it drops from. Every menu is one colour, over a card too; a submenu's first row is level with the row that opened it; Comments' order menu drops from its button; and menus, palettes and tips stand clear of the lines round a file.
- Every tab in the side pane has a mark, the Inbox, Comments, Preview, Problems and Log tabs too, and a program that names itself (`app=deploy`) names its tab. In a narrow pane the tabs behind the one in front are cut to their marks before the + after them goes, and their names come back as room allows. Editor, Task, Settings and side pane tabs share one shape: the same room before the mark, before the × and after it. The workspace tiles, the workspace picker and peek use the tabs' marks, and Cursor's agent has a mark of its own.
- A selection on a page reads as one shape: its lines touch, it runs under inline code and highlights, a list's bullets and numbers and a paper's section numbers are inside it, and a table's cells taken whole are filled from line to line. The buttons for a selection stand over its first line (over its paragraph or table when it starts inside one), clear of the words.
- Dialogs and cards have even room round them and before their buttons, and a box (the new file prompt, the branch chip, peek, Show Source, a button on a row) holds its words in its middle.
- A comment card has the same room on every side and starts at the pane's text edge, as the Agents list does. An agent's rewrite stands in a box of its own over quiet buttons, a reply sits under a hairline, and where Edit and Delete don't fit, ⋯ holds them. An agent's suggestion is a card with a dashed edge; the accent is the lit card's. Buttons on cards are named as in the palette: Ask Agent, Check My Review, Not Done.
- A kind's pill in the Comments pane is an outline, and the one you picked is filled, with × to show every kind again; the head says "Evidence · 1 of 4". An empty list with suggestions in the text says how many and that Next goes to each, and a long list has a scrollbar.
- What an agent took out is drawn in one colour for removed words everywhere: on a card's "Words gone" and in what changed.
- The Task page's divider is one line and its step control stands centred between its arrows.
- Nothing stands within 8 px of a line: the Find bar over a terminal, Show Source, tooltips, a tab's menu, the header chips in the docked pane, the conflict header's buttons.

### Fixed
- Quitting Cellix ended it with a crash: what came in with the quit (the release of the click on Stop and Quit, say) went to the workspace that had just closed. Your work was saved first, and the message went with the screen, so it went unseen, but Cellix ended with code 101. It ends with 0.
- Ctrl+click on a web address in a terminal's output looked for a file of that name. It opens the address.
- A selection in a terminal's output stayed on the same screen rows as the output scrolled. It stays on its words.
- A selected formula on a page wasn't drawn selected, and a paper's selected words were counted by Markdown's rules ("75 of 58 words").
- Letting go of a drag over the status bar left the selection stuck.
- Send Comments to Agent sent only comments on code: on a page it said there were none. It sends a document's comments too, and suggestions you kept.
- Inside Claude Code, the count of comments waiting and `/cellix-send` left out a page's comments and the suggestions you kept: with only those waiting, `/cellix-send` said none were. They count and send them now, through `cellix comments --waiting`, which lists everything no agent has had yet.
- A comment on words across lines held only the first line: Apply replaced that line with the whole rewrite, and only it was tinted. A comment holds the whole selection.
- When an agent reworded commented words, the card showed the new words as if you'd commented on them.
- Check My Review sent your comments to the agent twice; "Waiting for…" could name an agent other than the one asked; and the coverage strip counted suggestions you hadn't kept.
- In a paper, a `\replaced` whose words ran onto another line showed a stray brace and lost its new words on the page.
- With two agents reporting at once, a workspace's tile could say "done" while one of its tabs needed you. The tile, the Agents badge and the status bar count what every tab shows.
- A rule across a page was drawn in two tones; it's one hairline. A reviewer's note beside a line stood against the text in a box of its own; it stands apart, with the comment's mark and tint. A one-line preamble said "1 lines".
- A task's box on a page was a wide rectangle with its words against it. It's square, with room before the words.
- `->` in a code snippet written in the interface's font showed as an arrow. It shows as typed.
- A long path is cut at its start (`~/…/parent/name`), so its name stays; cut rows in Compare with and in search results end in "…" clear of their band.
- An install card could run out of the side pane, and Find over a list left an empty slot in its box and a stray line under the pane.
- A message in the status bar was cut short though the bar had room for it, and the word count said "1 words".
- The Git graph could put the branch you have checked out in its second lane when commits were made in the same second. Its first lane is always the checked-out branch's.
- A terminal's text ran to within a few pixels of its pane's right edge.
- The same file listed twice in Agent Context showed as picked twice.
- A large paste into a program that wasn't reading (a shell running a long command) held up all of Cellix until it read. Writes to programs wait their turn now, and a viewer that stops reading holds up neither the program nor anyone else watching it.
- A program that ended in the middle of a screen update left its pane frozen. The update ends after 150 ms.
- Moving the pointer steadily kept Cellix from noticing a file changed on disk.
- Turning smooth scrolling back on over SSH lost the slower pace scrolling keeps there.
- While an agent needed you, the file's details at the end of the status bar turned the colour that says so.
- Tests started by `harness/x` no longer leave sessions in your cache, or programs running after them.

## [0.10.2] - 2026-11-17

The new look, tightened after a first day in a real window: words in the middle of what holds them, less left over at the foot, and one measure for cards and buttons on Airy rows.

### Changed
- On Airy rows, words stand in the middle of what holds them: a text box, a button, a workspace's chip. They were raised, as they have to be on the terminal's own line to keep a tail off a box's edge, and on a taller row that put them plainly high.
- A workspace's chip, and a pill in the comments pane, is taller on Airy rows, with room under a tail.
- A card (the offer to install a language's support, the notice about logout, a stopped session) is closer round its words on Airy rows: a quarter of a row over its title and under its buttons, and no empty row between its words and its buttons.
- The bell and the power button at the foot are the size of the branch's mark beside them on Airy rows. They were two thirds of it.
- The bar at the foot stands in the middle of the space under the file, and there's less of that space. A row's height is fitted to your window, a pixel or two either side of what Row height asks, so the window's last pixels aren't all left under the bar: up to a whole row of them were.
- Where the workspaces have a row of their own under the status bar (a narrow window, or many workspaces), the two rows stand evenly in the space under the file: as much over the status bar as between the two and under the workspaces. Only the lower row had been moved down, and all of the space lay between them.
- A button at the end of a pointed row stands inside the row's highlight on Airy rows, with the highlight showing over and under it: Ask Agent on a problem, Install on a language server in Settings. It was exactly as tall as the highlight, and the two read as one shape.
- There's air between the views at the top of the sidebar and the heading under them (the folder's name and branch, OUTLINE, SOURCE CONTROL): as much over the heading as under it. It stood straight under the views' line.

### Fixed
- What the Outline says when there's nothing to list ran to the sidebar's edge and was cut there. It wraps, with room at the edge.
- What's under the pointer in the bar at the foot has the same room each side of it. The count of problems had its chip start on the mark's own edge and run well past the number, and was the one thing in the bar set at full size.
- Ask Agent on a row of the Problems tab stood in a box taller than the row's highlight.
- On the terminal's own row height, a sliver of the branch's mark showed over the bar at the foot.
- A tooltip over the bar at the foot had part of its shadow drawn apart from it, lower down. On Airy rows it stands at the top of its row, with all of its shadow under it.
- A problem's message too long for its row was cut at whatever letter came last. It ends in an ellipsis, clear of Ask Agent on a pointed row.

## [0.10.1] - 2026-11-16

### Fixed
- After a dropped SSH connection, `cellix` at home offered one workspace of several, though each of the others carried on as it was once its folder was opened by hand. A workspace was only noted as open while Cellix was leaving, and a Cellix cut off before it finished, or killed, never got that far: the next one found only the workspaces with something still running in them. A workspace is noted as open from the moment it's opened, until its tile is closed. `cellix ls` lists the workspaces a running Cellix has open as well.

## [0.10.0] - 2026-11-16

A new look: more air, sheets on the terminal's own colour, and less chrome. Nothing about how Cellix is used changes but where three things are: the views, the workspaces, and Quit.

### Added
- Row height, under Settings › Appearance: Airy, Comfortable, or the Terminal's. In Ghostty and kitty a row of Cellix is no longer a line of your terminal. Cellix lays out rows of its own, 1.4 times the line unless you choose otherwise, so everything has room round it while words and icons stay the sizes they were. Your terminal's settings aren't touched, and it holds over SSH. Under tmux or Zellij, inside Claude Code's pane and on a terminal without pictures a row stays the terminal's line. `row_height` in settings is `airy`, `comfortable` or `terminal`. On rows of its own the wheel moves a row at a time.
- A file's icon says what kind it is: marks of their own for Rust, Python, Markdown, git's files, JSON, configs and other code, and a page for the rest, each in its kind's colour. Folders are filled. They're in the tree, Quick Open, Source Control and on a comment's place.
- A file's tab has that icon before its name.
- The tree has a line down from each open folder, so what's inside a long one is plainly its own.
- A page of Settings with more on it than the window holds has a thumb at its edge, Appearance and Editor's list of language servers.

### Changed
- The file and the side pane are sheets on the frame, with round corners and a column of the frame between them. The frame is your terminal's own colour: the sidebar, the views and the bar at the foot. There were three surfaces, each ruled off from the next.
- The views (Explorer, Search, Outline, Source Control, Agent Context, and Settings at the end) run across the top of the sidebar, level with the tabs, with a line under the one in front. The rail down the left edge is gone, and the file has its five columns. With the sidebar closed the views go with it, and a button at the start of the tabs brings both back. On a terminal without pictures the rail stays.
- One bar at the foot. The workspaces lead the status bar's own row, the one in front in the accent, then the branch and what was last done, and at the far end the file's details, the machine's name and the bell. A window gets a row back. Where the two don't fit, in a narrow window or with many workspaces, the workspaces keep a row of their own under the status bar, as before.
- Quit is the power button at the far end of the bar at the foot, after the bell. It was an × at the top right, in the side pane's tab row, where it read as closing the pane or the workspace. Ctrl+Q quits as it did, and the first screen has the button in the same place.
- The tab in front, a file's or the side pane's, is marked by a line under it, and a line runs under the views, the file's tabs and the side pane's tabs alike.
- The tree has a column for its chevrons, so an open folder's no longer touches its icon, and names start a column later.
- The icons at the top right end a column short of the window's edge.
- A message in the bar at the foot that's too long for its room takes the room of the file's details, and of the cursor's place if it needs that too, so its end isn't cut off where a shorter bar would have held it. They're back at the next key, when the message goes.

## [0.9.2] - 2026-11-16

### Changed
- Words and icons are larger: the sizes VS Code has. In a terminal set to 14 pt, words are 13 pt where they were 11, beside code at 14. An icon beside a name is 16 pt where it was 13, the icons at the top right are 13 pt where they were 10, and the rail's are 24 pt where they were 15, on a rail one column wider with two rows to an icon. A page's words grew with the rest. Code keeps the terminal's own size. Words a sixth wider mean a long name in the sidebar is cut a few letters sooner.
- What's one row tall grew to hold those words: text boxes, the band behind a picked row in a palette or a menu, the workspace chips, the pills in the comments pane. In a text box the words stand a little higher, so the tail of a "g" keeps off its lower edge.
- A box, a pill or a card with a line round it is drawn on whole pixels, so its line is as sharp on top as at its sides.
- What explains a setting is in small words, as the design has it, which lets the longest of them fit a window of 120 columns.
- The row of emoji offered for a name keeps "Tab" at its end, the key that takes the first of them, and gives up its last name when the pane is narrow. It gave up the key.
- A check in Agent Context has three rows where it had two, which holds "never in AGENTS.md" and "nothing is loaded there" at the larger words.

### Fixed
- The side pane couldn't be given the whole window, or sized by dragging, where Cellix has little room: inside Claude Code, which gives it at most 45% of the window, or in any window under about 110 columns. The ⤢ after the pane's tabs gave way when the row was short, which a pane 40 columns wide always is; it keeps its place now, and the tabs' names are cut a little sooner for it. The line between the file and the pane was held to the widths the pane opens at, which left it a few columns to move in; dragged, it goes on until either side has 24 columns.
- A formula in a line of a page that stands taller than the line, a fraction of sums, lay over the words of the line under it. The line keeps an empty row under it now.
- A name in Agent Context that fitted its row lost the end of its last letter, "docs/STYLE.mc".
- The Timeline's heading was cut with "…" though the whole name fitted.
- The card that offers to install a language's server, "Install Terraform support?", had its title on its top edge and its buttons on its bottom one. It's the same kind of card as the logout notice now: half a row clear over the title and under the buttons, and a row between its words and its buttons. A tool with a long name gets a second line, where its sentence was cut at the card's edge on a terminal without pictures. While it installs the card keeps its size. Its middle button reads Not Now, as the logout notice's does.
- A button that isn't the main one, on one of those cards or in a dialog, changes under the pointer: lighter in the dark theme, deeper in the light one. It took a colour a level or two from the one it rests in, so nothing seemed to happen.
- A card in the window's corner that lay over the side pane had a strip of the file's colour above it and below it. Its round corners show what's under them.

## [0.9.1] - 2026-11-15

### Changed
- The Keyboard page in Settings lists the commands under their categories: each category is a heading, with its commands under it by name alone, and the one you've scrolled into stays at the top of the list. Every row used to carry its category, Agent: Ask Agent, which on a terminal without pictures left 19 of the 138 shortened in a 120-column window; one is now. What the filter finds keeps its whole title.

### Removed
- "Agent: Show Agents" is gone from the command palette and the Keyboard page. It opened or closed the side pane, which "View: Toggle Side Pane" does, and stood beside "Agent: List Agents" with nothing to tell the two apart. Keys set for it in settings still run it.

## [0.9.0] - 2026-11-15

### Changed
- The folder finder (+ New Workspace) says what its keys do while you type: "Tab goes into it · Enter opens it" at the end of the field, with a folder picked in the list. Tab already went into the picked folder to carry on with the ones inside it, but only the hint in the empty field said so, and that goes with the first letter typed.
- Every command has its category before its name, as VS Code's have: `Git: Stash Changes`, `View: Toggle Sidebar`, `Agent: Ask Agent`. The command palette lists them by that, so a category's commands stand together, and the Keyboard page in Settings names them the same way. Type a name as before, or a category to see what it holds: `git` lists Git's commands and `git hist` finds Show Git History. Keys set in `settings.json` under a command's name alone still work.
- Commands that match what you typed equally well stay in the palette's order, so a category's stay together: `op` lists Agent's three Opens, then File's, Preferences' and Workspace's. A name of sixteen letters or more used to drop below shorter ones.
- The Keyboard page in Settings gives a command's name half the page, where it had two fifths, since the name now has its category before it; one that still doesn't fit ends in an ellipsis. It used to be cut where the column ended.
- The command palette lists the commands that have something to act on now, as VS Code's does, where it listed all of them: Abort Merge with nothing being merged, six ways to settle a conflict in a file with none, Run Cell on a Rust file. What a file needs is there with a file in front, and by its kind: Format, Track Changes and Export to PDF on Markdown and papers, Build PDF on a paper, the notebook's commands on a notebook, Fold and Toggle Word Wrap on code, Go to Definition and the like where a language server runs. The merge conflict commands are there in a file that has conflicts, Abort Merge while a merge or rebase is under way, the stash commands that need a stash when there is one, Git's commands in a repository, Next Workspace with more than one open, Next Tab with more than one tab, Go Back with somewhere to go back to, and an agent's plan and proposed change while one waits. A command that isn't listed still runs by its keys, and says what's missing as before.

## [0.8.0] - 2026-11-15

### Added
- Paste a path into Quick Open (Ctrl+P) and Enter goes there. It's read the way it was copied: with a line after it (`src/app.rs:120`, `src/app.rs:120:8`, `app.rs#L120`, `app.rs(120,8)`), from the top of the disk or from `~`, between quotes, backticks or brackets, as a `file://` link, as an agent's `@src/app.rs`, as a diff's `b/src/app.rs`, with the rest of a compiler's or a search's line after it, or in the middle of a sentence. A file the path names is the first row even when the list wouldn't have it: one `.gitignore` leaves out, one outside the folder, or one the walk of a big folder hasn't reached yet. Typing a place works the same, so `app:120` opens the file you'd have picked at line 120, and `:120` alone goes to that line of the file in view.
- Quick Open lists the files you last looked at first, under "Recently opened", with the one before this at the top: Ctrl+P and Enter goes back to it, and again comes back. The folder's other files follow under "Files". It listed the folder in name order, whatever you'd been working on.
- `>` in Quick Open lists the commands, as it does in VS Code, and deleting it lists the files again. The hint in the empty field names what it takes: `:line`, `@`, `#` and `>`.
- Ctrl+V pastes into Quick Open, the command palette, a prompt and the Find bar. It did nothing there.
- What you type in Quick Open or the command palette is edited like any text: Left, Right, Home and End move the caret, Option or Ctrl with them moves by word, Delete and Option+Backspace work, and a click puts the caret at the letter clicked. Only letters and Backspace did anything before, at the end. A query longer than the box shows the part the caret is in.
- Ctrl+P opens Quick Open while the keyboard is in the side pane's agent or terminal, as Ctrl+Shift+P opens the commands from there. It went to the program in the pane. Pick a file and the keyboard goes to it.

### Changed
- The command palette lists the commands you last ran from it first, under "Recently used", the last at the top, so Ctrl+Shift+P and Enter runs it again. Every other command follows by name. They were in the order they'd been added to Cellix, Save first and Quit last. The recent ones are kept for the folder between runs.
- A file in Quick Open is its icon, its name, and then its folder, quieter, as VS Code lists one. It was the whole path as one run of words. The letters you typed are picked out in the row, in a file's name or folder and in a command's name, and they're looked for in the name before the folders. When a line follows the name, the picked row says where it will open, "line 120". Open Recent, the folder finder and the picture lists have icons too.
- Quick Open (Ctrl+P), the command palette (Ctrl+Shift+P) and every list like them have a box around what you type, as the Find bar has. The words stood nearer the panel's top than the line under them. The box is in the middle of its space now: the air above it, between it and the list, and under the last row is the same. What you type starts where the rows' words start: after the search icon in a list of files or symbols, whose rows have icons, and with no icon in the commands and the other plain lists. The picked row's band is the one menus have, a file's folder stays quieter than its name on it, and a list with nothing in it says "No matches" in place of closing up. In the light theme the box is a step darker than the white panel it's in.

### Fixed
- A click on a palette's own field, or on a group's name in its list, closed the palette. Only a click outside it does.
- A list in Quick Open, the command palette or Go to Symbol showed its first twelve rows and never moved: the arrows could pick a row past them that wasn't on screen, and the wheel did nothing. The list follows the picked row, Page Up and Page Down move a screen of it, the wheel scrolls it, and a thumb at its edge shows there's more and where you are in it.
- A paste while Quick Open or a prompt was open went into the commit message or the comment being written under it, when one of those had the keyboard before. The list or box on top takes it.
- A long path in Quick Open was cut at its end, where the file's name is. The name is whole now, and a folder too long for the row loses its middle.
- A command with a long name ran under its keys at the row's end. Its words stop short of them.
- On a terminal without pictures the picked row of Quick Open or the command palette looked like every other row. It has the picked colour behind it there too.

## [0.7.1] - 2026-11-14

### Changed
- The Comments tab's order and its resolved comments each have a button with a chevron, where there were plain words. **Show N resolved** now stays where it is when you click it: it stands after the comments still open and the resolved ones unfold under it, so the same click folds them away again. They used to be mixed in among the open ones, with the button at the very end of the list.
- Click words that have a comment while the side pane is open on another tab, an agent or the Preview say, and the Comments tab comes forward with that comment lit and in view. The tab you were on keeps its place in the row. A pane that's closed stays closed.

### Fixed
- A long list of comments froze the window for a while after the Comments tab was scrolled. Every turn of the wheel laid the whole list out again, as did every click and every frame, and a trackpad goes on sending turns after the list has stopped, so the window answered nothing until they had all been gone through. Past about 20,000 words of comments and answers each layout also measured all the text from scratch: 0.7 s a turn for 300 comments. The list is laid out once, when something in it changes, and a turn now takes under a millisecond.
- With every comment resolved, the "Show resolved" button was drawn on top of the hint under "No open comments.", and with the resolved ones showing, the first of them was. The hint has a row of its own.

## [0.7.0] - 2026-11-14

### Added
- Click text that has a comment and its card is lit in the Comments tab, which scrolls to it. Click a comment's card and its words are selected in the file, with the keyboard there.
- Alt+A asks the agent about what's selected, as the Ask Agent button does. The Comment and Ask Agent buttons by a selection show their keys in brackets, "Comment (Alt+C)" and "Ask Agent (Alt+A)", or whichever keys you've set.
- The Comments tab can list in the document's order, newest first or oldest first. The order is on a row under the kinds; click it to change it.
- A link in a comment or a reply is underlined in the accent colour, and a click opens it: in the side pane's browser where there is one, in your own browser otherwise.
- Emoji in comments. Type a name between colons, as on GitHub, and it becomes its emoji when you type the second colon: `:+1:` is 👍 and `:100:` is 💯. Part of a name, such as `:roc`, lists the emoji that fit under what you're writing: Tab takes the first, a click any of them. The eight you used last stand under a comment as you write it, a click each, and the face after them opens every emoji, searched by name. Emoji keep their colours where Cellix draws the text itself, and one press of Backspace takes a whole one. They're drawn in your machine's own emoji font, and where it has none, as a server over SSH rarely does, in Twemoji, which is now in the binary (about 1.5 MB). `CELLIX_EMOJI=built-in` uses Twemoji everywhere.
- Pictures in comments. While you write a comment or a reply, paste a picture, drop its file on the window, press Ctrl+V with one on the clipboard, or use the picture button under your words to take one from the project or the clipboard. It's kept in the project under `.cellix/pasted`, out of git, and the comment names it by its path, so the agent can open it. On the card the picture stands under your words, and a click opens it in the Preview. A terminal without pictures shows its name as a link that opens it.

### Changed
- A comment being written stands in a box, with the emoji and picture buttons inside it and two buttons under it: one that sends it, named for what that does (Comment, Reply, Save or Ask agent), and Cancel. Enter and Esc still do the same. While you answer or change a comment, those two take the place of its own buttons.

### Fixed
- A comment's card that ran past the bottom of the Comments pane was painted over the status bar under it. The pane's cards stay inside the pane.
- After the window had changed size, a click could land three or four lines above the pointer, low in the window, until another resize happened to put it right. Cellix guessed the height of a row from the window's size, which is off in terminals whose window is taller than its rows. It now asks the terminal for the exact size again after any resize.

## [0.6.8] - 2026-11-13

### Changed
- Over SSH, dragging the side pane's edge moves a line that follows the pointer, and the file and the pane resize once, when you let go. Resizing at every step sent the whole window again each time, so the edge jumped after the pointer. Without SSH the two sides still follow as you drag, and the edge is drawn in the accent colour while you hold it.

## [0.6.7] - 2026-11-13

### Fixed
- A side pane hidden while it filled the window came back at its usual width. It comes back filling the window, as it was.

## [0.6.6] - 2026-11-13

### Added
- Settings can be searched. A box at the top of the page looks through every page at once: the settings and commands that fit are listed together, each with its page's name, and each page in the list at the left shows how many it has. Before, only the Keyboard page could be searched.
- Find works in Review and on the Task page. Ctrl+F there looks through every change Review shows, or the task's timeline and the step beside it, marks what it finds and scrolls to it. In Review it used to open the project's Search, which is still Ctrl+Shift+F.
- Find works in the side pane's lists too. Click in Comments, the Inbox, Problems, Agents or a build's log and press Ctrl+F: it looks through everything the list holds, marks what it finds, and Enter scrolls to the next. A click on a file takes Find back to the file.
- Find works in the side pane. With the keyboard in a terminal's tab, Ctrl+F looks through everything the terminal has printed, its scrollback included: matches are marked, Enter and Shift+Enter step through them, and new output is searched as it arrives. In the tab of an agent that keeps its own screen, such as Claude Code, it opens the agent's transcript and searches that, which is the whole chat.

### Fixed
- On a merge conflict, the count of conflicts and the words on Compare, Both and Current were hard to read on the bar's tint. They read clearly now.
- What a stopped terminal last printed was dimmed too far to read.
- In the light theme a line stands between the sidebar and the file, which on a white terminal were almost one colour.
- A line of an integration's card in Settings was cut mid-word at its button. It ends in an ellipsis.
- A stopped terminal's pane showed three shades behind its card. It's one colour.
- A hidden side pane leaves a button in the corner, where the one that hides it stands, which shows it again as it was. Before, it came back only through the icon of a view.
- The first screen, where Cellix lists the workspaces that are open, had no way out: no button, and no key but the ones that pick. It now has Quit beside Open and the × in its corner, and Ctrl+Q works there. Quitting from it leaves every workspace as it is.

### Changed
- The Task page is a tab among the files', named "Task ·" and the task. It stays in the strip while you look at a file, a click brings it back where you left it, and its × closes it. Before, it took the file's place with no tab, and nothing showed how to get back to it.
- One look for the whole app, from the end-to-end design. Tabs are one shape for files and for the side pane: the strip is the frame's colour, and the tab in front takes the colour of what's under it with the accent's line on top. The side pane is one colour whatever it shows, a terminal included. The dark theme takes Catppuccin Mocha's colours for code and for added, changed and removed, and a slightly bluer text. The light theme keeps Latte's surfaces, with text and code colours deepened so each reads at 4.5 to 1.
- More room, and the same room everywhere. Every button is sized by one rule: its words keep a column and a half either side, and buttons side by side stand two columns apart. Before, a dialog, a comment, Review and the Task page each sized theirs differently. Menus, the command palette and prompts set their text two columns in, as dialogs did. The Find bar and prompts keep clear air around their boxes. Section headings are bold in every panel.
- Buttons are a little taller in their row, so their words have room above and below.
- Commit in Source Control is as wide as its words, not the whole sidebar, with Sync beside it. The buttons on a merge conflict, Abort, and the ones in Settings are sized the same way.
- In Settings a language server that isn't installed says Install in plain words, and shows a button when you point at its row.
- The workspace tiles, the branch beside the sidebar's title, the keys in Settings and the sidebar's Search boxes have more room. The Problems list starts a row under its tab, as the other lists do.
- Comments stand on raised cards that start a column from the pane's edge. Edit and Delete are plain words at the card's far end, apart from Reply, Ask agent and Resolve. The count and the kinds line up with the pane's text edge.
- In the Problems tab, Ask Agent is an ordinary button on the row under the pointer.
- With no file open, the Comments tab puts each file's comments under a heading with the file's name and how many there are. A click on a heading opens the file. Before, the cards didn't say which file they were on.
- The Comment and Ask Agent buttons by a selection in a terminal no longer cover its output. They stand on the row above the selection when that row's text ends before them, on the row below otherwise, and after the end of the selection's last row when neither has room.
- In the Agents panel a worktree's row says Review under the pointer, since that's what a click opens. The list of what can start is headed Start, with the terminal set apart from the agents.

## [0.6.5] - 2026-11-10

### Added
- A page keeps to a reading width. Its text is set to 88 columns at most, about 140 characters a line, and in a window wider than that the page stands in the middle. Tables and code keep to it too. Reading width under Settings › Editor, or `reading_width: false`, gives pages the whole pane back. CSV and TSV sheets always use the whole pane.
- A table with more columns than fit, and a long line in a block of code, scroll sideways on a page: tip the wheel across, or hold Shift and turn it, with the pointer over the block. Each has a thin scrollbar of its own, and the rest of the page stays put.
- The command palette shows the keys each command has now, including ones set under Settings › Keyboard.

### Changed
- Tables in Markdown and LaTeX keep to the page. Columns share its width and a long cell wraps onto rows of its own, instead of the table running on over the side pane. Ordinary words and numbers stay whole; a long link breaks after a slash.
- A narrow side pane keeps every tab in its row: the one in front whole, the others cut short with a "…". Hover one for its name, or click it to bring it forward. Before, only the tab in front showed. A pane too narrow even for that still lists its tabs under a ⌄.
- Every list in the side pane (Agents, Comments, Inbox, Problems, the build log) scrolls the same way, to its end and no further. The Agents list didn't scroll at all.
- The Agents panel lists Start an Agent above the worktrees, so a long list of worktrees can't push it out of view.
- The palette puts commands with a word that starts with what you typed first: "op" finds Open Folder before Pop Stash.
- Cellix does less while nothing is happening: it stops copying unsaved files twice a second to see whether they changed, asks git for committed text only when HEAD or the open files change, and reads a quiet terminal or agent 25 times a second rather than 125.

### Fixed
- Clicking a comment in the Comments pane with no file open now opens its file at the comment.
- A Cellix whose terminal went away without a hang-up kept running at full CPU. It now ends.
- A session host whose socket was gone kept its program running where nothing could reach it. It now ends with its program.
- A turn's checkpoint could still be lost when several agents prompted at the same moment.

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
