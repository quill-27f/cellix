#!/usr/bin/env python3
"""Walks through every surface of the real binary and writes a PNG of each, for a person or
a design reviewer to look at. No window opens, and everything runs against a throwaway
project and home folder, so a tour can't touch real settings.

    tour.py <binary> <out folder>
"""
import json, os, shutil, subprocess, sys, tempfile

binary, out = sys.argv[1], os.path.abspath(sys.argv[2])
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(out, exist_ok=True)
CTRL = lambda c: chr(ord(c) - 96)
RAIL_X = 1
# One row each since the rail's redesign, Files on the first.
SEARCH, OUTLINE, REVIEW = 2, 3, 5  # a row and a half apart, at their middles
LIST_AGENTS = [["keys", "\x1b[112;6u"], ["wait", 0.3], ["keys", "List Agents"], ["wait", 0.3], ["keys", "\r"]]

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


NOTE = r"""# Attention Is a Weighted Average

We study a model where each output is $y_i = \sum_{j} \alpha_{ij} x_j$ and the weights sum to one, for inputs of $n$ tokens.

## Scaled dot product

The weights come from a softmax over similarity scores:

$$
\alpha_{ij} = \frac{\exp\left(q_i^\top k_j / \sqrt{d}\right)}{\sum_{\ell=1}^{n} \exp\left(q_i^\top k_\ell / \sqrt{d}\right)}
$$

For a matrix $A \in \mathbb{R}^{n \times d}$ the cost is $O(n^2 d)$.

$$
\begin{aligned}
\nabla_\theta \mathcal{L} &= \mathbb{E}\left[ \nabla_\theta \log p_\theta(x) \right] \\
\sigma^2 &= \frac{1}{n} \sum_{i=1}^{n} (x_i - \bar{x})^2
\end{aligned}
$$

## Results

| Model | Accuracy | Params |
|---|---|---|
| Baseline | 91.2 | 1.2M |
| Attention | **94.8** | 1.4M |

- [x] Reproduce the baseline
- [ ] Add a learning curve
"""
PAPER = r"""\documentclass{article}
\usepackage{amsmath,graphicx}
\title{Attention Is a Weighted Average}
\author{Ada Lovelace}
\begin{document}
\maketitle
\begin{abstract}
We show that scaled dot product attention is a weighted average, and measure what that costs.
\end{abstract}

\section{Introduction}\label{sec:intro}
Each output is $y_i = \sum_j \alpha_{ij} x_j$, as in~\cite{knuth84}. The weights are \emph{learned} and sum to one, see Equation~\ref{eq:weights}.

\begin{equation}
  \alpha_{ij} = \frac{\exp(q_i^\top k_j / \sqrt{d})}{\sum_{\ell} \exp(q_i^\top k_\ell / \sqrt{d})}
  \label{eq:weights}
\end{equation}

\section{Results}
\begin{itemize}
  \item Accuracy improves by \textbf{3.6} points.
  \item The cost grows with $n^2$.
\end{itemize}

\begin{table}[h]
  \centering
  \begin{tabular}{lrr}
    \hline
    Model & Accuracy & Params \\
    \hline
    Baseline & 91.2 & 1.2M \\
    Attention & 94.8 & 1.4M \\
    \hline
  \end{tabular}
  \caption{Accuracy on the test set.}
\end{table}

\end{document}
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
    with open(f"{root}/docs/PLAN.md", "a") as f:
        f.write("\nBound the queue.\n")
    # What a build leaves, which .gitignore names: dimmed in the tree.
    os.makedirs(f"{root}/target/debug")
    with open(f"{root}/target/debug/queue", "w") as f:
        f.write("built\n")


def chart(path, width=1600, height=900):
    """A wide plot as a PNG, for the Preview: two falling curves on a grid."""
    import math, struct, zlib
    rows = []
    for y in range(height):
        row = bytearray(b"\xf5\xf5\xf7" * width)
        if y % 150 == 0:
            row = bytearray(b"\xd4\xd4\xdc" * width)
        rows.append(row)
    for x in range(width):
        for colour, lift in ((b"\x1e\x66\xf5", 0), (b"\xfe\x64\x0b", 40)):
            y = int(80 + lift + (height - 260) * (1 - math.exp(-x / 300)))
            for dy in range(-3, 4):
                rows[min(height - 1, y + dy)][x * 3:x * 3 + 3] = colour
    raw = b"".join(b"\x00" + bytes(row) for row in rows)
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header))
        f.write(chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def shoot(name, steps, *args, size="120x40", background="1e1e2e", cwd=None, env=None, timeout=70):
    """Runs one scene and writes `<name>.png`, with `steps` run first."""
    # TOUR_ONLY=50-review,02-code retakes just those, for a quick look after a change.
    only = os.environ.get("TOUR_ONLY")
    if only and name not in only.split(","):
        return
    script = [*steps, ["wait", 0.3], ["shot", f"{out}/{name}.png"]]
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(script, f)
    extra = env or {}
    env = {
        **os.environ,
        "HOME": HOME,
        "TERM_SIZE": size,
        "TERM_BG": background,
        "TERM_PADDING": "10x24",
        **extra,
    }
    env.pop("CLAUDE_CONFIG_DIR", None)
    command = ["python3", "-I", f"{here}/term.py", f.name, binary, *args]
    subprocess.run(command, cwd=cwd or ROOT, env=env, timeout=timeout, stdout=subprocess.DEVNULL)
    os.unlink(f.name)
    print(name, flush=True)


def workspaces(scratch):
    """Work left running in other projects, as the bar along the bottom shows it, and what
    stopped with the machine. Sessions need a host, whose socket needs a short path."""
    cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
    space = {"env": {"XDG_CACHE_HOME": cache, "SHELL": "/bin/sh"}}
    others = {}
    for name in ["api-server", "infra", "notes"]:
        others[name] = f"{scratch}/{name}"
        os.makedirs(others[name])
        with open(f"{others[name]}/README.md", "w") as f:
            f.write(f"# {name}\n")
    terminal = [["keys", CTRL("j")], ["wait", 1.0]]
    # Notes had a terminal when the machine went down.
    shoot("w0-notes", [*terminal, ["keys", "echo building the index\r"], ["wait", 0.5]],
          cwd=others["notes"], **space)
    subprocess.run(["pkill", "-9", "-f", cache])
    # api-server's terminal is busy; infra's is back at its prompt, its agent done.
    shoot("w0-api-server", [*terminal, ["keys", "sleep 300\r"], ["wait", 0.5]],
          cwd=others["api-server"], **space)
    shoot("w0-infra", [*terminal, ["keys", "echo deployed\r"], ["wait", 0.5]],
          cwd=others["infra"], **space)

    def hook(kind, root, **fields):
        stdin = json.dumps({"session_id": "s1", "cwd": root, **fields})
        return ["run", f"'{binary}' hook {kind}", stdin]

    told = [
        ["wait", 1.0],
        hook("notify", others["api-server"], message="Claude Code needs your permission"),
        hook("stop", others["infra"]),
        ["wait", 0.6],
    ]
    shoot("100-workspaces", told, CODE_FILE, size="150x44", **space)
    shoot("105-new-workspace-hover", [*told, ["move", 56, 43], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    shoot("106-folder-finder", [*told, ["click", 56, 43], ["wait", 0.6]], CODE_FILE, size="150x44", **space)
    shoot("107-folder-path", [*told, ["click", 56, 43], ["keys", "../in"], ["wait", 0.6]], CODE_FILE,
          size="150x44", **space)
    shoot("101-workspace-hover", [*told, ["move", 40, 43], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    # Its tile brings back what stopped, with what it last showed.
    shoot("102-stopped-card", [["wait", 1.0], ["keys", "\x1b[6;7~"], ["keys", "\x1b[6;7~"],
                                ["keys", "\x1b[6;7~"], ["wait", 1.0]], CODE_FILE, size="150x44", **space)
    # Started at home, nothing is open yet: what's running comes first.
    shoot("103-running-here", [["wait", 1.0]], cwd=HOME, size="150x44", **space)
    lingering = {"env": {**space["env"], "CELLIX_LOGOUT_STOPS": "1"}}
    shoot("104-logout-card", [*terminal, ["wait", 0.6]], CODE_FILE, size="150x44", **lingering)
    if os.environ.get("TOUR_KEEP"):
        print("kept", cache, flush=True)
        return
    subprocess.run(["pkill", "-9", "-f", cache])
    shutil.rmtree(cache, ignore_errors=True)


with tempfile.TemporaryDirectory() as scratch:
    ROOT = f"{scratch}/queue"
    HOME = f"{scratch}/home"
    os.makedirs(HOME)
    project(ROOT)
    CODE_FILE, PAGE_FILE = "src/queue.rs", "README.md"
    NOTE_FILE, PAPER_FILE = "docs/attention.md", "docs/main.tex"
    os.makedirs(f"{ROOT}/docs", exist_ok=True)
    for name, text in [(NOTE_FILE, NOTE), (PAPER_FILE, PAPER)]:
        with open(f"{ROOT}/{name}", "w") as f:
            f.write(text)
    with open(f"{ROOT}/docs/refs.bib", "w") as f:
        f.write("@book{knuth84,\n  title={The TeXbook},\n}\n")
    # Building needs TeX's packages, which only the real home folder has fetched.
    real = os.path.expanduser("~/Library/Caches/Tectonic")
    if os.path.isdir(real):
        os.makedirs(f"{HOME}/Library/Caches", exist_ok=True)
        os.symlink(real, f"{HOME}/Library/Caches/Tectonic")
    # Where the terminal's output and the Problems tab are, at 150x44 and 170x44.
    OUTPUT_FROM, OUTPUT_TO, OUTPUT_ASK = (97, 7), (125, 10), (111, 6)
    PROBLEM_COUNT, PROBLEM_ROW = (9, 42), (150, 2)
    GEAR = (RAIL_X, 37)
    settings = [["click", *GEAR]]

    workspaces(scratch)
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
    replacing = [["keys", CTRL("r")], ["keys", "item"], ["keys", "\t"], ["keys", "entry"]]
    shoot("11-replace", replacing, CODE_FILE)
    shoot("12-search-switches", [["click", RAIL_X, SEARCH], ["keys", "item"], ["keys", "\x1bw"], ["wait", 0.6]], CODE_FILE)
    shoot("13-review", [["keys", "\u0007"], ["wait", 1.2]], CODE_FILE)
    shoot("13-strip-hover", [["wait", 0.8], ["move", 95, 0], ["wait", 0.4]], CODE_FILE)
    shoot("13-agents-click", [["wait", 1.5], ["click", 111, 0], ["wait", 1.5]], CODE_FILE)
    shoot("13-quit-hover", [["wait", 6], ["move", 119, 0], ["wait", 1.2]], CODE_FILE)
    shoot("13-quit-beside-pane", [["wait", 6], ["click", 111, 0], ["wait", 3.5], ["move", 116, 0], ["wait", 1.2]], CODE_FILE)
    git_tab = [["click", RAIL_X, REVIEW], ["wait", 0.8]]
    shoot("13-source-hover", [*git_tab, ["move", 14, 11], ["wait", 0.3]], CODE_FILE)
    shoot("13-source-staged", [*git_tab, ["click", 28, 11], ["wait", 0.8], ["move", 14, 11]], CODE_FILE)
    shoot("13-source-diff", [*git_tab, ["click", 14, 11], ["wait", 1.2]], CODE_FILE)
    shoot("13-stacked-pane", [*LIST_AGENTS, ["wait", 0.6]], CODE_FILE, size="72x44")
    shoot("13-source-control", [["click", RAIL_X, REVIEW], ["wait", 0.8]], CODE_FILE)
    shoot("13-source-message", [["click", RAIL_X, REVIEW], ["wait", 0.8], ["click", 14, 3],
                                ["keys", "Say what changed and why"], ["move", 14, 15], ["wait", 0.3]], CODE_FILE)
    shoot("14-agents", [*LIST_AGENTS, ["wait", 0.5]], CODE_FILE)
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
    shoot("27-hover-rail", [["move", RAIL_X, REVIEW]], CODE_FILE)
    shoot("28-hover-tree", [["move", 12, 3]], CODE_FILE)
    shoot("29-hover-tab", [["move", 38, 0]], CODE_FILE)
    shoot("30-hover-menu", [["right", 60, 12], ["move", 64, 19]], CODE_FILE)
    shoot("31-hover-palette", [["keys", CTRL("p")], ["move", 60, 7]], CODE_FILE)
    shoot("32-hover-toolbar", [["move", 100, 1]], PAGE_FILE)
    shoot("33-hover-branch", [["move", 3, 39]], CODE_FILE)
    shoot("34-hover-header", [["move", 18, 0]], CODE_FILE)
    shoot("35-hover-review", [["keys", "\u0007"], ["wait", 0.6], ["move", 12, 2]], CODE_FILE)
    shoot("36-hover-accept", [["keys", "\u0007"], ["wait", 0.6], ["move", 114, 1]], CODE_FILE)
    shoot("37-hover-settings", [*settings, ["click", 45, 4], ["move", 80, 16]], CODE_FILE)
    shoot("38-hover-dialog", [["keys", "x"], ["keys", CTRL("q")], ["move", 70, 17]], CODE_FILE)
    shoot("39-hover-agents", [*LIST_AGENTS, ["wait", 0.5], ["move", 90, 12]], CODE_FILE)
    shoot("41-hover-tab-close", [["move", 42, 0]], CODE_FILE)
    shoot("42-hover-install", [*settings, ["click", 45, 4], ["move", 113, 16]], CODE_FILE)
    shoot("43-light-hover-menu", [["right", 60, 12], ["move", 64, 19]], CODE_FILE, background="eff1f5")
    shoot("40-hover-find", [["keys", CTRL("f")], ["keys", "item"], ["move", 112, 3]], CODE_FILE)

    FORMULA, TABLE = (77, 1), (80, 1)
    shoot("44-note", [], NOTE_FILE, size="150x44", background="eff1f5")
    shoot("45-note-dark", [], NOTE_FILE, size="150x44")
    shoot("46-formula-open", [["click", 90, 13], ["wait", 0.5]], NOTE_FILE, size="150x44")
    shoot("47-math-panel", [["click", *FORMULA]], NOTE_FILE, size="150x44", background="eff1f5")
    shoot("48-table-picker", [["click", *TABLE], ["move", 100, 7]], NOTE_FILE, size="150x44")
    shoot("49-table-menu", [["right", 41, 29]], NOTE_FILE, size="150x44", background="eff1f5")
    # A debug build presents a page of math slowly, so each step is given time to land.
    comment = [["wait", 1.5], ["click", 90, 5], ["wait", 0.6], ["keys", "\\eOP"], ["wait", 0.6]]
    comment += [["keys", "add comment"], ["keys", "\r"], ["wait", 0.6]]
    comment += [["keys", "Is this true for multi-head attention?\r"], ["wait", 1.0]]
    shoot("50-comments", comment, NOTE_FILE, size="150x44", background="eff1f5")
    # Being typed: the caret stands after the space just typed.
    typing = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], ["keys", "\\eOP"], ["wait", 1.5],
              ["keys", "add comment"], ["wait", 1.0], ["keys", "\r"], ["wait", 1.5],
              ["keys", "Is this true for "], ["wait", 1.5]]
    shoot("50-comment-typing", typing, NOTE_FILE, size="150x44", background="eff1f5")
    # A review the agent has checked: a verdict on the reviewer's comment and a suggestion.
    review = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "Only with softmax weights. Say so.",
         "replies": [{"author": "Claude", "text": "Agree (Correctness): other normalisers needn't sum to one."}],
         "resolved": False, "told": 1, "asked": False, "kind": "Correctness", "suggested_by": ""},
        {"id": 2, "file": NOTE_FILE, "line": 3, "quote": "for inputs of $n$ tokens",
         "text": "Say how large n gets in the experiments, or the cost claim can't be judged.",
         "replies": [], "resolved": False, "told": 0, "asked": False, "kind": "Evidence",
         "suggested_by": "Claude"},
        # A plan sent again: one comment followed its words, one is on words it dropped.
        {"id": 3, "file": "docs/PLAN.md", "line": 3, "quote": "Bound the queue.",
         "text": "At what size?", "replies": [{"author": "Claude", "text": "A thousand items."}],
         "resolved": False, "told": 1, "asked": False, "kind": "", "suggested_by": ""},
        {"id": 4, "file": "docs/PLAN.md", "line": 3, "quote": "Cache every read first.",
         "text": "Measure before caching.", "replies": [], "resolved": False, "told": 0,
         "asked": False, "kind": "", "suggested_by": "", "earlier": 1},
        # And one on words it reworded: quoted as said, marked as changed.
        {"id": 5, "file": "docs/PLAN.md", "line": 3, "quote": "Retry three times on a timeout.",
         "text": "Why three?", "replies": [], "resolved": False, "told": 0, "asked": False,
         "kind": "Reproducibility", "suggested_by": "", "earlier": 1, "reworded": True},
        # Settled, so it's under "Show 1 resolved" rather than on the page.
        {"id": 6, "file": NOTE_FILE, "line": 1, "quote": "Attention",
         "text": "Cite the original paper.", "replies": [], "resolved": True, "told": 0,
         "asked": False, "kind": "", "suggested_by": ""},
    ]
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(review, f)
    shoot("50-review", [["wait", 1.5], ["keys", "\\eOP"], ["wait", 0.6], ["keys", "show comments"],
                        ["wait", 3.0], ["keys", "\r"], ["wait", 3.0]],
          NOTE_FILE, size="150x44")
    shoot("50-plan-versions", [["wait", 1.0], ["keys", "\\eOP"], ["wait", 0.6], ["keys", "show comments"],
                               ["wait", 1.0], ["keys", "\r"], ["wait", 1.5]],
          "docs/PLAN.md", size="150x44")
    # Narrow beside the editor: a pill that can't share its header's row goes under it.
    shoot("50-plan-versions-narrow", [["wait", 1.0], ["keys", "\\eOP"], ["wait", 0.6],
                                      ["keys", "show comments"], ["wait", 1.0], ["keys", "\r"],
                                      ["wait", 1.5]],
          "docs/PLAN.md", size="96x44")
    shoot("51-agent-list", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="150x44")
    shell = [*LIST_AGENTS, ["wait", 0.6]]
    shoot("52-paper", [], PAPER_FILE, size="150x44", background="eff1f5")
    shoot("53-paper-dark", [], PAPER_FILE, size="150x44")
    build = [["click", 164, 1], ["wait", 45]]
    shoot("54-paper-pdf", build, PAPER_FILE, size="170x44", background="eff1f5")
    shoot("55-paper-cite", [["click", 69, 1]], PAPER_FILE, size="150x44", background="eff1f5")
    shoot("55-paper-menu", [["right", 70, 14]], PAPER_FILE, size="150x44", background="eff1f5")
    shoot("56-narrow-note", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="100x34")
    # About the size Claude Code docks Cellix at: at most 45% of a window, beside the chat.
    shoot("57-dock-page", [["wait", 1.5]], NOTE_FILE, size="64x40")
    shoot("57-dock-comments", [["wait", 1.5], ["keys", "\\eOP"], ["wait", 0.6], ["keys", "show comments"],
                               ["keys", "\r"], ["wait", 1.0]], NOTE_FILE, size="64x40")
    shoot("57-dock-agents", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="64x40")
    shoot("57-dock-menu", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4]], NOTE_FILE, size="64x40")
    shoot("57-dock-menu-format", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4], ["move", 24, 16],
                                  ["wait", 0.4]], NOTE_FILE, size="64x40")
    # With the pane open the note is taller than its view, so it has an end to scroll past.
    shoot("57-dock-end", [*LIST_AGENTS, ["wait", 0.6], ["wheel", 30, 10, 40], ["wait", 1.0]], NOTE_FILE,
          size="64x40")

    # The Preview: an image beside the page that shows it.
    os.makedirs(f"{ROOT}/figures", exist_ok=True)
    chart(f"{ROOT}/figures/loss.png")
    RESULTS, LOSS = "docs/results.md", "figures/loss.png"
    with open(f"{ROOT}/{RESULTS}", "w") as f:
        f.write("# Training loss\n\nThe loss settles after 40k steps.\n\n![Loss by step](../figures/loss.png)\n")
    shoot("90-preview-image", [["wait", 1.0]], RESULTS, LOSS, size="150x44")
    # Zoomed past the pane's width: the thumb under it, and the hint on moving around.
    ZOOM_IN = (142, 1)
    # A debug build reads the picture slowly, and each zoom reads it again.
    zoomed = [["wait", 2.0]] + [["click", *ZOOM_IN], ["wait", 1.0]] * 4 + [["wait", 1.0]]
    shoot("91-preview-zoomed", zoomed, RESULTS, LOSS, size="150x44")
    # The pane's maximize button, at the end of its own tabs, and the window it takes.
    MAXIMIZE = (114, 0)
    shoot("93-pane-maximized", [["wait", 1.0], ["click", *MAXIMIZE], ["wait", 0.6], ["move", 80, 20]],
          RESULTS, LOSS, size="150x44")
    shoot("94-pane-restore-hover", [["wait", 1.0], ["click", *MAXIMIZE], ["wait", 0.6], ["move", 80, 20],
                                    ["wait", 0.3], ["move", 22, 0]], RESULTS, LOSS, size="150x44")
    shoot("95-editor-maximized", [["wait", 4.0], ["double", 36, 0], ["wait", 0.6]], CODE_FILE, LOSS,
          size="150x44")
    # A tab dragged: held over the right half of the file, and the pane dragged under it.
    TAB = (36, 0)
    shoot("96-tab-drag-split", [["wait", 1.0], ["press", *TAB], ["hold", 70, 12], ["hold", 112, 20]],
          CODE_FILE, RESULTS, size="150x44")
    shoot("97-tab-drag-reorder", [["wait", 1.0], ["press", 50, 0], ["hold", 40, 0], ["hold", 34, 0]],
          CODE_FILE, RESULTS, size="150x44")
    PREVIEW_TAB = (100, 0)
    shoot("98-pane-drag-under", [["wait", 1.0], ["press", *PREVIEW_TAB], ["hold", 80, 20],
                                 ["hold", 60, 34]], RESULTS, LOSS, size="150x44")
    # A debug build takes a while over the frame that moves the whole layout.
    shoot("99-pane-under", [["wait", 2.0], ["drag", *PREVIEW_TAB, 60, 34], ["wait", 5.0]],
          RESULTS, LOSS, size="150x44")
    shoot("92-preview-panned", [*zoomed, ["drag", 125, 20, 105, 14], ["wait", 0.6]], RESULTS, LOSS,
          size="150x44")

    # The samples, which use everything a page can do.
    samples = os.path.join(ROOT, "samples")
    shutil.copytree(os.path.join(here, "..", "..", "samples"), samples)
    WRITING = os.path.join(samples, "writing.md")
    PAPER = os.path.join(samples, "paper", "main.tex")
    light = {"size": "150x44", "background": "eff1f5"}
    shoot("57-welcome", [], **light)
    start_shell = [["keys", "\x1b[112;6u"], ["wait", 0.3], ["keys", "New Terminal"], ["wait", 0.3], ["keys", "\r"], ["wait", 1.5]]
    shoot("58-welcome-with-shell", start_shell, **light)
    shoot("59-sample-top", [], WRITING, **light)
    for number, (name, notches) in enumerate(
        [
            ("lists", 3),
            ("callouts", 6),
            ("table", 9),
            ("math", 12),
            ("pictures", 16),
            ("flow", 22),
            ("class", 30),
            ("charts", 40),
            ("tikz", 52),
            ("code", 60),
            ("footnotes", 68),
            ("changes", 74),
        ]
    ):
        steps = [["wheel", 90, 20, notches], ["wait", 1.0]]
        shoot(f"{60 + number}-sample-{name}", steps, WRITING, **light)
    shoot("72-sample-dark", [["wheel", 90, 20, 22], ["wait", 1.0]], WRITING, size="150x44")
    shoot("73-selection-bar", [["double", 60, 7], ["wait", 0.5]], WRITING, **light)
    shoot("74-outline", [["click", RAIL_X, OUTLINE], ["wait", 0.8]], WRITING, **light)
    shoot("75-data", [], os.path.join(samples, "data.csv"), **light)
    shoot("76-notebook", [], os.path.join(samples, "notebook.ipynb"), **light)
    shoot("77-paper-top", [["wait", 1.0]], PAPER, **light)
    for number, (name, notches) in enumerate(
        [("math", 8), ("theorems", 14), ("drawn", 24), ("tables", 32), ("changes", 40)]
    ):
        steps = [["wait", 1.0], ["wheel", 90, 20, notches], ["wait", 1.0]]
        shoot(f"{78 + number}-paper-{name}", steps, PAPER, **light)

    # Last, so the broken paper they leave open is in no other scene.
    # A failing run in a terminal, its output selected: the buttons that comment on it or ask
    # the agent about it, as by a selection in a file.
    # Each scene has a cache of its own, or the second rejoins the first one's shell.
    in_shell = lambda cache: {"XDG_CACHE_HOME": cache, "SHELL": "/bin/sh", "PS1": "$ ", "ENV": ""}
    caches = [tempfile.mkdtemp(prefix="cx", dir="/tmp") for _ in range(2)]
    failing = ("printf 'test refuses_once_full ... FAILED\\n\\nthread panicked at src/queue.rs:16:9:\\n"
               "assertion failed: queue.push(4).is_err()\\n  left: Ok(())\\n right: Err(Full)\\n'\r")
    selected = [["keys", CTRL("j")], ["wait", 1.5], ["keys", failing], ["wait", 1.0],
                ["drag", *OUTPUT_FROM, *OUTPUT_TO], ["wait", 0.5]]
    shoot("108-terminal-ask", selected, CODE_FILE, size="150x44", env=in_shell(caches[0]))
    shoot("108-terminal-ask-hover", [*selected, ["move", *OUTPUT_ASK], ["wait", 0.4]], CODE_FILE,
          size="150x44", env=in_shell(caches[1]))
    for cache in caches:
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
    # A paper that doesn't build: its problem in the list, the pointer on it.
    with open(f"{ROOT}/docs/broken.tex", "w") as f:
        f.write("\\documentclass{article}\n\\begin{document}\nHello \\wrongmacro{} world.\n\\end{document}\n")
    # A loaded machine can take over a minute to build, so this one gets longer.
    problems = [["wait", 2.0], ["click", 164, 1], ["wait", 100], ["click", *PROBLEM_COUNT], ["wait", 0.6],
                ["move", *PROBLEM_ROW], ["wait", 0.4]]
    shoot("109-problem-ask", problems, "docs/broken.tex", size="170x44", timeout=130)
