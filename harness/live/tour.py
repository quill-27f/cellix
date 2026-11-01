#!/usr/bin/env python3
"""Walks through every surface of the real binary and writes a PNG of each, for a person or
a design reviewer to look at. No window opens, and everything runs against a throwaway
project and home folder, so a tour can't touch real settings.

    tour.py <binary> <out folder>
"""
import json, os, subprocess, sys, tempfile

binary, out = sys.argv[1], os.path.abspath(sys.argv[2])
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(out, exist_ok=True)
CTRL = lambda c: chr(ord(c) - 96)
RAIL_X = 2
SEARCH, REVIEW, AGENTS = 4, 7, 10

CODE = """//! A bounded queue.
use std::collections::VecDeque;

/// Refuses work once it's full, so a fast producer can't outrun its consumer.
pub struct Queue<T> {
    items: VecDeque<T>,
    limit: usize,
}

impl<T> Queue<T> {
    pub fn new(limit: usize) -> Self {
        Self { items: VecDeque::new(), limit }
    }

    pub fn push(&mut self, item: T) -> Result<(), Full> {
        if self.items.len() == self.limit {
            return Err(Full);
        }
        self.items.push_back(item);
        Ok(())
    }
}
"""
PAGE = """# AdPilot

An AI agent that runs Google and Meta ads for small businesses. The plan is in `docs/PLAN.md`, and progress is tracked in `docs/STATUS.md`.

## Standing rules

- **Rust wherever practical**, in a Cargo workspace.
- **Guardrails live in code**, never in prompts. Only `crates/core` may call write APIs.
- [ ] Check the spend cap
- [x] Dry-run by default

> Tests in core are never weakened to make code pass.

| Name | Size |
|---|---|
| Queue | 22 lines |

## Workflow

Work one milestone at a time. Propose the design first, then implement.

```rust
fn main() {}
```
"""


def git(root, *args):
    identity = ["-c", "user.name=tour", "-c", "user.email=tour@example.com"]
    subprocess.run(["git", *identity, *args], cwd=root, check=True, capture_output=True)


def project(root):
    os.makedirs(f"{root}/src")
    os.makedirs(f"{root}/docs")
    files = {
        "src/queue.rs": CODE,
        "README.md": PAGE,
        "docs/PLAN.md": "# Plan\n",
        "Cargo.toml": '[package]\nname = "queue"\n',
        ".gitignore": "target\n",
    }
    for name, text in files.items():
        with open(f"{root}/{name}", "w") as f:
            f.write(text)
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "First")
    git(root, "branch", "feature/queue")
    git(root, "branch", "fix/scroll")
    # Something to review.
    with open(f"{root}/src/queue.rs", "a") as f:
        f.write("\n/// Returned when the queue is full.\npub struct Full;\n")


def shoot(name, steps, *args, size="120x40", background="1e1e2e"):
    """Runs one scene and writes `<name>.png`, with `steps` run first."""
    script = [*steps, ["wait", 0.3], ["shot", f"{out}/{name}.png"]]
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(script, f)
    env = {
        **os.environ,
        "HOME": HOME,
        "TERM_SIZE": size,
        "TERM_BG": background,
    }
    env.pop("CLAUDE_CONFIG_DIR", None)
    command = ["python3", "-I", f"{here}/term.py", f.name, binary, *args]
    subprocess.run(command, cwd=ROOT, env=env, timeout=30, stdout=subprocess.DEVNULL)
    os.unlink(f.name)
    print(name, flush=True)


with tempfile.TemporaryDirectory() as scratch:
    ROOT = f"{scratch}/queue"
    HOME = f"{scratch}/home"
    os.makedirs(HOME)
    project(ROOT)
    CODE_FILE, PAGE_FILE = "src/queue.rs", "README.md"
    GEAR = (RAIL_X, 36)
    settings = [["click", *GEAR]]

    shoot("01-welcome", [])
    shoot("02-code", [], CODE_FILE)
    shoot("03-page", [], PAGE_FILE)
    shoot("04-menu-page", [["right", 60, 12]], PAGE_FILE)
    shoot("05-menu-code", [["right", 60, 12]], CODE_FILE)
    shoot("06-menu-explorer", [["right", 12, 2]])
    shoot("07-menu-tab", [["right", 36, 0]], CODE_FILE)
    shoot("08-open-file", [["keys", CTRL("p")]], CODE_FILE)
    shoot("09-commands", [["keys", "\\eOP"], ["keys", "op"]], CODE_FILE)
    shoot("10-branches", [["click", 3, 39]], CODE_FILE)
    shoot("11-find", [["keys", CTRL("f")], ["keys", "item"]], CODE_FILE)
    shoot("12-search", [["click", RAIL_X, SEARCH]], CODE_FILE)
    shoot("13-review", [["click", RAIL_X, REVIEW], ["wait", 0.4]], CODE_FILE)
    shoot("14-agents", [["click", RAIL_X, AGENTS], ["wait", 0.5]], CODE_FILE)
    shoot("15-settings-appearance", settings, CODE_FILE)
    shoot("16-settings-editor", [*settings, ["click", 45, 4]], CODE_FILE)
    shoot("17-settings-integrations", [*settings, ["click", 45, 5]], CODE_FILE)
    shoot("18-settings-keyboard", [*settings, ["click", 45, 6]], CODE_FILE)
    shoot("19-dialog", [["keys", "x"], ["keys", CTRL("q")]], CODE_FILE)
    shoot("20-prompt", [["right", 12, 2], ["click", 14, 2]])
    shoot("21-selection", [["double", 49, 13], ["keys", CTRL("d")]], CODE_FILE)
    shoot("22-narrow", [], CODE_FILE, size="64x30")
    shoot("23-narrow-menu", [["right", 20, 10]], PAGE_FILE, size="64x30")
    shoot("24-light-page", [], PAGE_FILE, background="eff1f5")
    shoot("25-light-menu", [["right", 60, 12]], PAGE_FILE, background="eff1f5")
    shoot("26-light-settings", settings, CODE_FILE, background="eff1f5")
