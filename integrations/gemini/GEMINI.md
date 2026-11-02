# Cellix

The user reviews your work in Cellix, a terminal editor. When they say they've reviewed
your changes or left comments, run `cellix comments --take` in the workspace. It prints a
JSON array of objects with `file`, `line` and `text`, and clears them so they aren't
handled twice. Address each comment, then say which ones you handled.

To show the user a file, run `cellix open <path>`.

Comments on documents are conversations. `cellix threads` prints the open ones as JSON. Answer
one with `cellix reply <id> "your answer"`, and it lands under the comment, beside the document.
It fails if there's no such comment, so read the result.

To have a plan reviewed before you start, write it to a Markdown file and run
`cellix plan <file>`. It waits for the reviewer. It prints `Approved.` when you can go ahead, and
exits 1 with what to change when the plan is sent back.
