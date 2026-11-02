---
name: cellix-review
description: Read the reviewer's line comments from a running Cellix editor. Use when the user says they have reviewed your changes or left comments in Cellix.
---

Run `cellix comments --take` in the workspace. It prints a JSON array of objects with
`file`, `line` and `text`, and clears them so they are not handled twice.

Address each comment, then say which ones you handled. If the command reports no running
Cellix, ask the user to open one in this workspace.

To show the user a file, run `cellix open <path>`.

Comments on documents are conversations. `cellix threads` prints the open ones as JSON. Answer
one with `cellix reply <id> "your answer"`, and it lands under the comment, beside the document.
It fails if there's no such comment, so read the result.

A `<proposed_plan>` reaches the reviewer by itself when your turn ends. For any other plan, write
it to a Markdown file and run
`cellix plan <file>`. It waits for the reviewer. It prints `Approved.` when you can go ahead, and
exits 1 with what to change when the plan is sent back.
