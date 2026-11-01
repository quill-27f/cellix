---
name: cellix-review
description: Read the reviewer's line comments from a running Cellix editor. Use when the user says they have reviewed your changes or left comments in Cellix.
---

Run `cellix comments --take` in the workspace. It prints a JSON array of objects with
`file`, `line` and `text`, and clears them so they are not handled twice.

Address each comment, then say which ones you handled. If the command reports no running
Cellix, ask the user to open one in this workspace.

To show the user a file, run `cellix open <path>`.
