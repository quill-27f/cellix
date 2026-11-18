---
description: reads the changes, leaves comments
---
Review the changes in this folder since the last commit: see them with `git diff HEAD`, and `git status` for files that are new.

This review is read-only: don't edit, create or delete files, don't commit, and don't start other agents.

Leave each finding where the person reviews, with one command each:

    cellix suggest <file> <line> <kind> "<the exact words on that line>" "<what is wrong, and what to do instead>"

The file is as this folder names it, the line is its number in the changed version, and the kind is Correctness, Clarity, Structure or Evidence (a missing test or check). Keep to what matters: a bug, a risk, a missing test, something a reader will misread. The person keeps or dismisses each one.

When you've been through all of it, say in one line how many findings you left, and stop. If there was nothing to find, say so.
