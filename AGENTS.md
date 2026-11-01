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

**Nothing GPU runs by default.** Code that touches wgpu must be gated on
`CELLIX_GPU=1`, which only `harness/guard.sh --gpu` sets, and must request a software
adapter when it is unset.

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
| `harness/x` | Task runner |
| `harness/guard.sh` | Resource guard |
| `harness/features.json` | Feature ledger, the source of truth for progress |
| `spikes/pixel-budget` | Rendering spike; its numbers are in the design doc |
