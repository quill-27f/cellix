# Cellix

The user reviews your work in Cellix, a terminal editor. When they say they've reviewed
your changes or left comments, run `cellix comments --take` in the workspace. It prints a
JSON array of objects with `file`, `line` and `text`, and clears them so they aren't
handled twice. Address each comment, then say which ones you handled.

To show the user a file, run `cellix open <path>`.
