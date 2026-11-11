# Working on Cellix

Cellix is a terminal-native, mouse-first editor for reviewing and correcting agent
work. Read `docs/DESIGN.md` for what it is and why. This file says how to work here.

## Start of a session

1. `harness/x status`: machine state and progress.
2. Read the last entry of `docs/PROGRESS.md`.
3. `harness/x next`: the next feature. Work on one at a time, in order.

## Rules

**Never run `cargo build`, `test`, `check`, `clippy` or `run` directly.** Use
`harness/x`, or `harness/guard.sh -- <command>` for anything else that is heavy. This
machine is shared with other work and is often under memory pressure. A hook refuses
direct runs. The guard runs one job at a time, across all worktrees, and may wait for
memory. Exit 75 means it was never admitted, 137 that it was killed for memory or GPU
use, and 124 that it timed out. Treat 137 as a bug to find. Do not raise budgets to
get past it.

**Nothing GPU runs by default.** Cellix has no GPU code. Windows opened for real-terminal
checks, as in `harness/visual.sh` and `harness/live/scroll.sh`, go through
`harness/guard.sh --gpu`. They take focus, so anything the user types lands in them.

**A feature is done when a test proves it.** Set `"passes": true` in
`harness/features.json` only when the test named there exists and passes, and that
test drives real behaviour. `harness/x verify` fails if the test is missing. Never
weaken, skip or delete a test to pass. If a feature's test is wrong, fix it and say so
in `docs/PROGRESS.md`.

**Test through the outside when you can.** Prefer, in order:
- `App` tests: send input events and assert on the rendered `Grid`.
- PTY tests in `crates/cellix/tests`: the real binary, driven by keys and mouse.
- `harness/x mux`: the real binary inside tmux and Zellij.

To see the interface, run `harness/x snapshot` and read the PNG. It needs no terminal or
window. `harness/visual.sh` opens real windows, so use it only to check terminal behaviour.

**Look at every surface before calling a UI change done.** `harness/x tour` drives the
real binary through a headless terminal and writes a PNG of each surface to `target/tour`,
going through the tile cache and smooth scrolling as a real terminal does. Beside each PNG
is the screen's text, and `manifest.json` says which commit each picture is of. A scene
waits for words its surface shows, and one that never gets there fails rather than leave a
picture of the wrong screen: read the failures before the pictures. `TOUR_ONLY=a,b` retakes
scenes. Then have the `ui-expert` agent review those pictures, and fix what it finds. Tests
have repeatedly passed on things that looked wrong.

**Every feature must work on the cell tier.** All UI draws to the `Grid`. Backends only
present it, so a feature never talks to a backend directly.

## Finishing a change

1. `harness/x verify` passes.
2. Commit one feature per commit, with a plain message saying what changed and why.
3. Add a short entry to `docs/PROGRESS.md`: what was done, what is unverified, what is next.

If a design decision changes, update `docs/DESIGN.md` in the same commit.

## Map

| Path | Holds |
|---|---|
| `crates/` | The workspace; `docs/DESIGN.md` lists each crate's role |
| `crates/cellix/integrations` | The Claude Code, Codex and Gemini CLI plugins, built into the binary |
| `harness/x` | Task runner |
| `harness/guard.sh` | Resource guard |
| `harness/features.json` | Feature ledger, the source of truth for progress |
| `spikes/pixel-budget` | Rendering spike; its numbers are in the design doc |
