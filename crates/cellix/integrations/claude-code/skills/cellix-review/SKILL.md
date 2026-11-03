---
name: cellix-review
description: Read the reviewer's line comments from a running Cellix editor. Use when the user says they have reviewed your changes or left comments in Cellix.
---

Run `cellix comments --take` in the workspace. It prints a JSON array of objects with
`file`, `line`, `text`, `quote` (the words or line the comment is about) and `removed` (true when
the line is one your change took out, so `line` counts in the old version), and clears them so
they are not handled twice.

Address each comment, then say which ones you handled. If the command reports no running
Cellix, ask the user to open one in this workspace.

To show the user a file, run `cellix open <path>`.

Comments on documents are conversations. `cellix threads` prints the open ones as JSON. Answer
one with `cellix reply <id> "your answer"`, and it lands under the comment, beside the document.
It fails if there's no such comment, so read the result.

To have a plan reviewed before you start, write it to a Markdown file and run
`cellix plan <file>`. It waits for the reviewer. It prints `Approved.` when you can go ahead, and
exits 1 with what to change when the plan is sent back.

When the reviewer tracks changes in a document, your edits to it show to them as suggestions to
accept or reject, so edit the file as usual. To suggest a rewrite without making it, write it in
the document as a suggestion: in Markdown `{~~old~>new~~}`, `{++added++}` and `{--removed--}`,
and in LaTeX `\replaced{new}{old}`, `\added{text}` and `\deleted{text}`. Cellix shows each one
with Accept and Reject.

When the reviewer asks you to check their review, answer each of their comments with
`cellix reply <id> "Agree (<kind>): <why>"` or `cellix reply <id> "Disagree (<kind>): <why>"`, where
<kind> is Clarity, Correctness, Evidence, Structure or Reproducibility. Then suggest comments they
missed with `cellix suggest <file> <line> <kind> "<exact words>" "<comment>"`, favouring kinds they
haven't covered. Suggestions appear with Keep and Dismiss; don't edit the document yourself.

When a comment asks for different wording, you can answer with the words themselves:
`cellix reply <id> --suggest "<new words for the quoted text>"`. The reviewer sees Apply, which puts
your rewrite in the document as a suggestion to accept or reject.
