#!/usr/bin/env python3
"""Walks through every surface of the real binary and writes a PNG of each, for a person or
a design reviewer to look at. No window opens, and everything runs against a throwaway
project and home folder, so a tour can't touch real settings.

    tour.py <binary> <out folder>

Beside each PNG goes the screen's text, as `<name>.txt`. A scene can say what its surface
shows (`expect=`), and steps can wait for words rather than a clock (`["until", "words"]`): a
scene that never gets there writes no picture, is listed as failed, and fails the tour. A
picture of the wrong screen has passed for the right one before, so a picture of one colour,
one whose screen said nothing, and one the same as the scene's before it though their steps
differ are taken again, and fail the scene, with no picture left, if they come back so.
`manifest.json` says, for
each scene, which commit its picture is of, when it was taken, and whether it failed, so a
folder of pictures from several runs still says how old each one is.

TOUR_ONLY=a,b takes just those scenes. TOUR_PART=2/3 takes the second third of them all, in
order: the whole tour is longer than one guarded job may run. TOUR_LIST=1 names the scenes
it would take, and takes none.
"""
import hashlib, json, os, shutil, struct, subprocess, sys, tempfile, time, zlib

binary, out = sys.argv[1], os.path.abspath(sys.argv[2])
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(out, exist_ok=True)
CTRL = lambda c: chr(ord(c) - 96)
# A dark terminal's background: herdr's frame, #101114, which the dark theme stands on.
DARK = "101114"
# Where each view's icon is, across the top of the sidebar: three columns each from one in.
SEARCH, OUTLINE, REVIEW = (5, 0), (8, 0), (11, 0)
# The command palette, opened with Ctrl+Shift+P or F1 and waited for, then a command typed
# and run, and the palette gone with what was typed into it. Each step waits for the screen to
# say the one before it landed: keys sent before the palette is up are typed into the file.
PALETTE_UP = "Type to search"
# A place named by what the screen says there: term.py finds it, so a step still lands when
# the layout moves. [words, columns right of their first letter, rows down, which row].
ON = lambda words, right=1, down=0, which=0: [[words, right, down, which]]
# In the bar at the foot, where words can be on a row over it too: the last row that says them.
FOOT = lambda words, right=1: ON(words, right, 0, -1)
# The face under a comment being written in a 150-column window, after the eight emoji. By
# its column: an emoji is one letter of the screen's text and two cells of the screen, so
# words after one aren't where the text has them.
EMOJI_BUTTON = (123, 5)
# The button that shows the resolved comments, when none is open: under the two lines that
# say so, in that window. And where it stands under the review's two cards, and the order's
# button over them.
SHOW_RESOLVED = ON("Show 2")
# Not Now, on the card that offers a language's server.
INSTALL_NOT_NOW = ON("Not Now")
REVIEW_RESOLVED, REVIEW_ORDER = ON("Show 1"), ON("Document", 2)
def command(words, opened="\x1b[112;6u"):
    return [["keys", opened], ["until", PALETTE_UP], ["keys", words], ["until", words], ["keys", "\r"],
            ["gone", words]]
F1 = "\\eOP"
LIST_AGENTS = [*command("List Agents"), ["until", "WORKTREES"]]
SHOW_INBOX = command("Show Inbox")
# A word each scene's surface shows, where one can be named: the scene's picture is only
# taken once the screen says it. Hover scenes and plain pages have none.
EXPECT = {
    "04-menu-page": "Paste",
    "05-menu-code": "Rename Symbol",
    "06-menu-explorer": "New File",
    "07-menu-tab": "Close Others",
    "08-open-file": "Search files",
    "08-open-file-path": "line 12",
    "08-open-file-recent": "RECENTLY OPENED",
    "09-commands": "Open Settings",
    "09-commands-recent": "RECENTLY USED",
    "09-commands-scrolled": "Open Folder as a Workspace",
    "10-branches": "Switch branch",
    "100-workspaces": "Resume",
    "108-install-card": "Install Terraform support?",
    "108-install-card-hover": "Install Terraform support?",
    "108-install-card-light": "Install Terraform support?",
    "108-install-card-pane": "Install Terraform support?",
    "101-workspace-hover": "Resume",
    "102-stopped-card": "building the index",
    "103-running-here": "workspaces are open",
    "104-logout-card": "when you disconnect",
    "105-new-workspace-hover": "Resume",
    "106-folder-finder": "Folder to open",
    "107-folder-path": "Enter opens it",
    "108-terminal-ask": "FAILED",
    "108-terminal-ask-hover": "FAILED",
    "108-terminal-find": "FAILED",
    "109-problem-ask": "Undefined control sequence",
    "11-replace": "entry",
    "12-search": "SEARCH",
    "12-search-switches": "results",
    "120-go-to-symbol": "structs",
    "121-go-to-symbol-typed": "methods",
    "122-workspace-symbols": "jobs.rs",
    "123-signature-help": "index: usize",
    "124-inlay-hints": "run.rs",
    "125-inlay-hints-light": "run.rs",
    "13-agents-click": "WORKTREES",
    "13-quit-beside-pane": "WORKTREES",
    "13-review": "Accept",
    "13-source-control": "SOURCE CONTROL",
    # The diff it opens: SOURCE CONTROL shows before the press too, and a loaded machine
    # can take more than a second to read what changed.
    "13-source-diff": "Change 1",
    "13-source-hover": "SOURCE CONTROL",
    "13-source-message": "what changed and",
    "13-source-message-select": "what changed and",
    "11-find-select": "1 of",
    "11-find-drag": "1 of",
    "11-find-select-light": "1 of",
    "136-settings-filter-select": " setting",
    "08-open-file-select": "src/queue",
    "13-source-staged": "STAGED CHANGES",
    "13-stacked-pane": "WORKTREES",
    "130-run-task": "Select the task",
    # What the task printed as it stopped: "make build" is in the task picker too, so a scene
    # whose Enter never ran it passed with the picker open.
    "131-task-failed": "exited with code",
    "132-task-problems": "undeclared identifier",
    "132-task-problem-hover": "Ask Agent",
    "133-open-recent": "recent folder",
    "134-settings-editor": "files save",
    "135-settings-keyboard": "command's keys",
    "136-settings-keys-search": " setting",
    "137-settings-keys-record": " setting",
    "138-welcome-recent": "RECENT",
    "14-agents": "WORKTREES",
    "140-inbox": "waiting",
    "141-inbox-hover": "waiting",
    "142-inbox-find": "1 of",
    "15-settings-appearance": "Changes apply",
    "150-task-page": "test run",
    "151-task-plan": "test run",
    "152-task-turn": "test run",
    "153-task-review-turn": "COMPARE WITH",
    "154-task-narrow": "Bound the queue",
    "16-settings-editor": "files save",
    "17-settings-integrations": "Cellix works with",
    "18-settings-keyboard": "command's keys",
    "19-dialog": "Save changes",
    "25-light-open-file": "queue.rs",
    "26-light-settings": "Changes apply",
    "30-hover-menu": "Rename Symbol",
    "31-hover-palette": "Search files",
    "35-hover-review": "Accept",
    "36-hover-accept": "Accept",
    "37-hover-settings": "files save",
    "38-hover-dialog": "Save changes",
    # Folding is ready (its chevrons show), and the agent's finding has come in.
    "110-view-fold-hover": "⌄",
    "176-review-suggested": "1 suggested",
    "176-review-suggested-light": "1 suggested",
    "39-hover-agents": "WORKTREES",
    "42-hover-install": "files save",
    # The pointed row says all it needs, in small letters as at rest.
    "42-settings-needs": "npm, or Homebrew",
    # One paragraph, set in pixels: a line's cells hold its first 42 letters.
    "108-install-card-needs": "Copies brew install",
    "43-light-hover-menu": "Rename Symbol",
    "50-comments": "yours",
    "50-comment-emoji": ":rock",
    "50-emoji-picker": "Search emoji",
    "50-emoji-picker-light": "fire",
    "50-comment-picture": "yours",
    "50-comment-picture-light": "yours",
    "50-comment-link-menu": "Copy Link",
    "50-comment-mentions": "Comment 1",
    "50-comment-selected": "same gap",
    "50-comment-copy-menu": "Copy Quote",
    "50-comment-copy-menu-light": "Copy Quote",
    "m1-agent-names": "Fix the login",
    "m1-agent-names-tip": "Fix the login",
    "m1-agent-names-list": "Refund route",
    "m1-agent-names-light": "Fix the login",
    "m1-agent-in-terminal": "Refund route",
    "m1-agent-in-terminal-light": "Refund route",
    "m1-agent-in-terminal-cells": "Refund route",
    "50-plan-versions": "yours",
    "57-flow-dock": "Addressed",
    "50-flow-next": "Comment 2 of",
    "50-flow-changes": "Changed since",
    "50-flow-kind": "Say how large",
    "50-flow-cards-light": "Addressed by",
    "50-flow-cards": "Addressed by",
    "50-flow-chips": "Reproducibility",
    "50-plan-versions-narrow": "yours",
    "50-review": "suggested",
    "50-review-resolved": "Hide 1",
    "50-review-resolved-light": "Hide 1",
    "50-review-order": "Newest first",
    "50-review-tab-menu": "Copy Review",
    "50-review-saved": "Saved the review",
    "50-comment-glance": "Open in Comments",
    "50-comment-glance-light": "Open in Comments",
    "50-comment-glance-open": "Check My",
    "50-comments-all-resolved": "No open comments",
    # A button's cells are fewer than its words, so less of it is waited for.
    "50-comments-resolved-shown": "Hide 2",
    "51-agent-list": "WORKTREES",
    "51-inbox-empty": "Nothing",
    "56-narrow-note": "WORKTREES",
    "57-dock-agents": "WORKTREES",
    "64-transcript-tab": "stand-in",
    "64-transcript-tab-full": "stand-in",
    "64-transcript-tab-hover": "stand-in",
    "64-transcript-tab-light": "stand-in",
    "64-transcript-tab-menu": "Open Trans",
    "65-transcript-page": "Show Source",
    "66-transcript-light": "Show Source",
    "67-transcript-scrolled": "Show Source",
    "74-outline": "OUTLINE",
    "75-data": "Show Source",
    "81-context": "AGENT CONTEXT",
    "81-context-codex": "What Codex would",
    "81-context-hover": "AGENT CONTEXT",
    "81-context-open": "AGENT CONTEXT",
    "81-git-graph": "SOURCE CONTROL",
    "82-git-commit-open": "SOURCE CONTROL",
    "83-git-timeline": "TIMELINE",
    "84-git-compare": "Compare the file",
    "85-git-compared": "COMPARED",
    "86-git-merge-panel": "Merging",
    "87-git-conflict": "conflict",
    "88-git-conflict-hover": "conflict",
    "89-git-stashes": "STASHES",
    "90-preview-image": "loss.png",
    "91-preview-zoomed": "loss.png",
    "92-preview-panned": "loss.png",
    "93-pane-maximized": "hidden while",
    "94-pane-restore-hover": "hidden while",
    "95-editor-maximized": "hidden while",
    "96-tab-drag-split": "split right",
    "98-pane-drag-under": "loss.png",
}
# About how many scenes there are, to cut the tour into parts of a size. Count them with
# TOUR_LIST=1 when scenes are added: too low a count puts every scene past it in the last part,
# which then runs out of the guard's time (0.11.0's tour lost ten scenes that way).
SCENES_IN_ALL = 305
# Scenes come to, shot, and those that never reached their surface.
SEEN, SHOT, FAILED = [0], [], []
# The scene shot last, with its picture's digest and what it was taken from; and this one's.
LAST = [None, None]
# Which commit the binary is, as far as git can say: the pictures are of that.
COMMIT = subprocess.run(["git", "-C", here, "describe", "--always", "--dirty"], capture_output=True,
                        text=True).stdout.strip()
MANIFEST = f"{out}/manifest.json"
try:
    with open(MANIFEST) as f:
        SCENES = json.load(f)["scenes"]
except (OSError, ValueError, KeyError):
    SCENES = {}


def wrong(name, steps, cells):
    """What says a scene's picture is of no screen, or of the one before: one colour, nothing
    in the screen's text, or the same picture as the scene shot before it from other steps.
    Each has passed for a surface before (a blank page while a sample loaded, a submenu's
    scene aimed at a rule that opened nothing and looked like the menu's). A scene on cells
    has only its text to go by: this terminal draws no letters, so its picture is its ground."""
    with open(f"{out}/{name}.png", "rb") as f:
        data = f.read()
    at, packed, width, height = 8, b"", 0, 0
    while at < len(data):
        size, kind = struct.unpack(">I4s", data[at:at + 8])
        if kind == b"IHDR":
            width, height = struct.unpack(">II", data[at + 8:at + 16])
        elif kind == b"IDAT":
            packed += data[at + 8:at + 8 + size]
        at += 12 + size
    pixels = zlib.decompress(packed)
    # term.py writes every row unfiltered, a zero and then its pixels.
    if not cells and pixels == (b"\0" + pixels[1:4] * width) * height:
        return "its picture is one colour"
    try:
        with open(f"{out}/{name}.txt", encoding="utf8") as f:
            said = f.read()
    except OSError:
        said = ""
    if not said.strip():
        return "the screen said nothing"
    digest = hashlib.sha1(pixels).hexdigest()
    before = LAST[0]
    if not cells and before and before[1] == digest and before[2] != steps:
        return f"its picture is {before[0]}'s, from other steps"
    LAST[1] = (name, digest, steps)
    return None


def note(name, why):
    """Writes down how a scene went, at once, so a tour cut short still says what it took."""
    SCENES[name] = {"commit": COMMIT, "at": time.strftime("%Y-%m-%d %H:%M:%S"), **({"failed": why} if why else {})}
    with open(MANIFEST, "w") as f:
        json.dump({"scenes": SCENES}, f, indent=1, sort_keys=True)

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
SCHEDULER = """//! Runs jobs in order, each when its time comes.
use std::collections::BinaryHeap;
use std::time::{Duration, Instant};

/// Holds jobs until they're due; a job that fails is tried again after a pause that doubles each time, up to a limit, and is then reported with the error it last gave.
pub struct Scheduler {
    queue: BinaryHeap<Job>,
    retries: u32,
}

impl Scheduler {
    pub fn new(retries: u32) -> Self {
        Self { queue: BinaryHeap::new(), retries }
    }

    pub fn run_due(&mut self, now: Instant) -> Vec<Result<(), String>> {
        let mut done = Vec::new();
        while let Some(job) = self.queue.peek() {
            if job.due > now {
                break;
            }
            let mut job = self.queue.pop().expect("peeked");
            match (job.work)() {
                Ok(()) => done.push(Ok(())),
                Err(error) if job.tries < self.retries => {
                    job.tries += 1;
                    job.due = now + Duration::from_millis(100 * 2u64.pow(job.tries));
                    self.queue.push(job);
                }
                Err(error) => done.push(Err(format!("{} failed after {} tries: {error}", job.name, job.tries))),
            }
        }
        done
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }
}

struct Job {
    name: String,
    due: Instant,
    tries: u32,
    work: Box<dyn FnMut() -> Result<(), String>>,
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


# A page with a table that wraps, one with more columns than fit, and a long line of code,
# for a window wide enough that the page keeps to its reading width.
WIDE = """# Reading width

A paragraph long enough to wrap in the column: """ + "the quick brown fox jumps over the lazy dog. " * 8 + """

| Setting | What it does |
|---|---|
| Reading width | """ + "Pages keep their text to a column in the middle of a wide pane. " * 3 + """|
| Minimap | The file at a glance. |

## A table with more columns than fit

| """ + " | ".join(f"col{i:02d}" for i in range(1, 31)) + """ |
|""" + "---|" * 30 + """
""" + "\n".join("| " + " | ".join(f"{r}.{i}" for i in range(1, 31)) + " |" for r in range(1, 5)) + """

## Code

```rust
fn main() {
    let total = """ + " + ".join(f"value_{i:02d}" for i in range(1, 40)) + """;
}
```

Text after the code.
"""


def git(root, *args, env=None):
    identity = ["-c", "user.name=tour", "-c", "user.email=tour@example.com"]
    subprocess.run(["git", *identity, *args], cwd=root, check=True, capture_output=True,
                   env={**os.environ, **(env or {})})


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


def shoot(name, steps, *args, size="120x40", background=DARK, cwd=None, env=None, timeout=70,
          expect=None, setup_for=(), cellix=False):
    """Runs one scene and writes `<name>.png` and `<name>.txt`, with `steps` run first. With
    `expect`, only once the screen says it.

    A scene starts with no `.cellix` in the project (`ROOT`): left by the scenes before, it
    showed as a folder in later pictures' Explorer, and a scene could stand on what another
    had left there. One whose caller has just written its own there says `cellix=True`. A
    scene that others stand on, as the workspaces' terminals the bar, the Inbox and a stopped
    tile show, names them in `setup_for`, and runs without a picture when only they're asked
    for."""
    # TOUR_ONLY=50-review,02-code retakes just those, for a quick look after a change.
    only = os.environ.get("TOUR_ONLY")
    listed = only.split(",") if only else []
    setup = bool(only) and name not in listed and any(other in listed for other in setup_for)
    if only and name not in listed and not setup:
        return
    if setup:
        print(f"setting up {name}", flush=True)
    index, SEEN[0] = SEEN[0], SEEN[0] + 1
    if not only and os.environ.get("TOUR_PART"):
        part, parts = (int(n) for n in os.environ["TOUR_PART"].split("/"))
        if min(index * parts // SCENES_IN_ALL, parts - 1) != part - 1:
            return
    if os.environ.get("TOUR_LIST"):
        print(name, flush=True)
        return
    expect = expect or EXPECT.get(name)
    waited = [["until", expect]] if expect else []
    # A scene run only for what it leaves has its picture taken where nobody looks.
    shots = tempfile.mkdtemp(prefix="cx-setup") if setup else out
    script = [*steps, *waited, ["wait", 0.3], ["shot", f"{shots}/{name}.png"]]
    # A picture from an earlier tour mustn't stand in for one this tour didn't take. One run
    # only as another's setup takes none, so its earlier picture stays: deleted here, w0-notes,
    # w0-api-server and w0-infra went from a tour when 142-inbox-find was taken again.
    for old in (f"{out}/{name}.png", f"{out}/{name}.txt"):
        if os.path.exists(old) and not setup:
            os.unlink(old)
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(script, f)
    extra = env or {}
    # Each scene gets a cache of its own, so tabs, carets and staged files one scene leaves
    # aren't restored into the next. Sessions need a host, whose socket needs a short path.
    own_cache = None if "XDG_CACHE_HOME" in extra else tempfile.mkdtemp(prefix="cx", dir="/tmp")
    if own_cache:
        extra = {**extra, "XDG_CACHE_HOME": own_cache}
    ours = os.path.realpath(cwd or ROOT) == os.path.realpath(ROOT) and not cellix
    said = tempfile.NamedTemporaryFile(suffix=".txt", delete=False)
    said.close()
    env = {
        **os.environ,
        "CELLIX_SCREEN_TEXT": said.name,
        # A stand-in, so no picture has the name of the machine it was taken on.
        "CELLIX_MACHINE": "dev-box",
        # And a plain shell with a plain prompt, so none has a login or a host in a terminal.
        "SHELL": "/bin/sh",
        "PS1": "$ ",
        "ENV": "",
        "HOME": HOME,
        # A stand-in login too: a card that names it (104's `loginctl enable-linger`) shows it.
        "USER": "dev",
        "LOGNAME": "dev",
        "TERM_SIZE": size,
        # Rows are Cellix's own, 1.4 lines of 36 pixels, as Settings has them until it's told
        # otherwise. A scene on cells has the terminal's lines. A press here counts in rows
        # of that height, so Cellix is told not to fit a row's height to the window, as it
        # does for a person. TOUR_ROW=52 takes scenes with rows fitted, for a window where
        # 52 is the height that fits (40 rows are 58 lines, and 52 px leaves 8 of them).
        **(
            {}
            if "CELLIX_TIER" in extra
            else {
                "TERM_ROW": os.environ.get("TOUR_ROW", "50"),
                "CELLIX_ROW_FIT": "1" if "TOUR_ROW" in os.environ else "0",
            }
        ),
        "TERM_BG": background,
        "TERM_PADDING": "10x24",
        # The caret held still, so it's in every picture of it, half a second away or not.
        # Settings still shows Cursor blinking on, as a person's does.
        "CELLIX_CURSOR_BLINK": "0",
        **extra,
    }
    env.pop("CLAUDE_CONFIG_DIR", None)
    # term.py stands for a person's Ghostty, so the tiles go as files it reads, as they do
    # there, even when an agent runs the tour from inside Claude Code.
    env.pop("CLAUDECODE", None)
    # By its own path, so a scene can have a PATH of its own.
    run = [sys.executable, "-I", f"{here}/term.py", f.name, binary, *args]
    # What the scene is taken from, so a picture the same as the last scene's is only wrong
    # when they were meant to differ.
    taken_from = json.dumps([steps, args, size, background], default=str)
    # Taken once more if it misses, or if its picture is of no screen or the last one's: a
    # loaded machine can lose a scene its place, and one that misses twice is worth a
    # person's look.
    for attempt in ("first", "second"):
        if ours:
            shutil.rmtree(f"{ROOT}/.cellix", ignore_errors=True)
        try:
            done = subprocess.run(run, cwd=cwd or ROOT, env=env, timeout=timeout, capture_output=True, text=True)
            why = done.stdout.strip().splitlines()[-1:] if done.returncode else None
        except subprocess.TimeoutExpired:
            why = [f"took more than {timeout} seconds"]
        if setup:
            shutil.rmtree(shots, ignore_errors=True)
            break
        if why is None and os.path.exists(f"{out}/{name}.png"):
            looked = wrong(name, taken_from, "CELLIX_TIER" in extra)
            why = [looked] if looked else None
            if looked:
                # A picture of the wrong screen isn't left to stand for the right one.
                for bad in (f"{out}/{name}.png", f"{out}/{name}.txt"):
                    if os.path.exists(bad):
                        os.unlink(bad)
        if why is None:
            break
        print(f"retaking {name}: {' '.join(why)}" if attempt == "first" else f"missed twice: {name}", flush=True)
        if own_cache:
            # A retake starts afresh too: kept in the cache, what the first try left (a tab,
            # the folder's consent to agents starting agents) changed what the second showed.
            subprocess.run(["pkill", "-9", "-f", own_cache])
            shutil.rmtree(own_cache, ignore_errors=True)
            own_cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
            env["XDG_CACHE_HOME"] = own_cache
    os.unlink(f.name)
    for left in (said.name, said.name[:-4] + ".new", said.name[:-4] + ".rows"):
        if os.path.exists(left):
            os.unlink(left)
    if own_cache:
        subprocess.run(["pkill", "-9", "-f", own_cache])
        shutil.rmtree(own_cache, ignore_errors=True)
    if setup:
        return
    if why is None and os.path.exists(f"{out}/{name}.png"):
        LAST[0] = LAST[1]
        SHOT.append(name)
        note(name, None)
        print(name, flush=True)
    else:
        why = " ".join(why or ["no picture was written"])
        FAILED.append(name)
        note(name, why)
        print(f"FAILED {name}: {why}", flush=True)


def workspaces(scratch):
    """Work left running in other projects, as the bar along the bottom shows it, and what
    stopped with the machine. Sessions need a host, whose socket needs a short path."""
    cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
    # Claude Code is a stand-in on PATH that says so and waits. Its news below comes through
    # Cellix's real hook to its tab: an agent with no tab here isn't listed, counted or shown.
    tools = f"{scratch}/space-bin"
    os.makedirs(tools)
    with open(f"{tools}/claude", "w") as f:
        f.write("#!/bin/sh\nprintf 'claude (stand-in)\\n'\nexec sleep 600\n")
    os.chmod(f"{tools}/claude", 0o755)
    space = {"env": {"XDG_CACHE_HOME": cache, "SHELL": "/bin/sh", "PATH": f"{tools}:{os.environ['PATH']}"}}
    others = {}
    for name in ["api-server", "infra", "notes"]:
        others[name] = f"{scratch}/{name}"
        os.makedirs(others[name])
        with open(f"{others[name]}/README.md", "w") as f:
            f.write(f"# {name}\n")
    terminal = [["keys", CTRL("j")], ["wait", 1.0]]
    claude = [*command("Open Claude Code", F1), ["until", "stand-in"], ["wait", 0.4]]
    # Each waits for what its terminal was told before going on, so one whose keys never
    # landed fails rather than leave a picture of the welcome page: w0-infra's did once, and
    # passed.
    # Every later scene here stands on what these three leave, and they run for any of them.
    group = ["100-workspaces", "105-new-workspace-hover", "106-folder-finder", "107-folder-path",
             "101-workspace-hover", "140-inbox", "141-inbox-hover", "142-inbox-find",
             "102-stopped-card", "103-running-here", "104-logout-card"]
    # Notes had a terminal when the machine went down.
    shoot("w0-notes", [*terminal, ["keys", "echo building the index\r"], ["wait", 0.5]],
          cwd=others["notes"], expect="building the index",
          setup_for=group, **space)
    subprocess.run(["pkill", "-9", "-f", cache])
    # api-server's terminal is busy, beside its agent; infra's is back at its prompt, beside
    # the agent that's about to finish its turn.
    shoot("w0-api-server", [*terminal, ["keys", "sleep 300\r"], ["until", "sleep 300"], ["wait", 0.5],
                            *claude], cwd=others["api-server"], setup_for=group, **space)
    shoot("w0-infra", [*terminal, ["keys", "echo deployed\r"], ["until", "deployed"], ["wait", 0.5],
                       *claude], cwd=others["infra"], setup_for=group, **space)

    def hook(kind, root, **fields):
        stdin = json.dumps({"session_id": "s1", "cwd": root, **fields})
        return ["run", f"'{binary}' hook {kind}", stdin]

    asking = hook("notify", others["api-server"], message="Claude Code needs your permission")
    told = [
        ["wait", 1.0],
        asking,
        hook("stop", others["infra"]),
        ["wait", 0.6],
    ]
    shoot("100-workspaces", told, CODE_FILE, size="150x44", **space)
    # New Workspace stands after the four tiles. Where the tiles lead the status bar's row it's
    # a + alone, its words in its tip, so it's found by the + with a space after it: the
    # words were looked for, and the scenes never reached it.
    new_workspace = FOOT("+ ", 0)
    shoot("105-new-workspace-hover", [*told, ["move", *new_workspace], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    shoot("106-folder-finder", [*told, ["click", *new_workspace], ["wait", 0.6]], CODE_FILE, size="150x44", **space)
    shoot("107-folder-path", [*told, ["click", *new_workspace], ["keys", "../in"], ["wait", 0.6]], CODE_FILE,
          size="150x44", **space)
    # A tile's name is cut short in the screen's text by the words after it, so less is asked for.
    shoot("101-workspace-hover", [*told, ["move", *FOOT("infr")], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    # What waits everywhere, in the side pane: an agent asking, a finished turn, a failed run.
    failed = hook("tool-failed", others["api-server"], tool_input={"command": "cargo test orders"},
                  tool_response={"stdout": "test refunds::rejects_twice ... FAILED\ntest result: FAILED. 13 passed; 1 failed"})
    # The agent's run fails, and then it asks again: news of a tool it ran after asking says
    # the ask was answered, so it asks last.
    inbox = [*told, failed, ["wait", 0.3], asking, ["wait", 0.6], *SHOW_INBOX, ["wait", 0.8]]
    shoot("140-inbox", inbox, CODE_FILE, size="150x44", **space)
    # The pointer on the failed run, and Find after a press under the list's heading, which
    # are there whatever else waits: the scenes waited for "finished its turn" and "waiting
    # in", which an agent with no tab in its workspace doesn't leave. The heading is found by
    # "waiting", the start of it: in the pane it's cut at "newest fir…".
    shoot("141-inbox-hover", [*inbox, ["move", *ON("tests failed")], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    # Find, after a press on the pane, looks through the list in front of it.
    find = [*inbox, ["click", *ON("waiting", 1, 1)], ["wait", 0.3], ["keys", CTRL("f")], ["wait", 0.4],
            ["keys", "server"], ["wait", 0.6]]
    shoot("142-inbox-find", find, CODE_FILE, size="150x44", **space)
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


# A stand-in for an agent that isn't installed: it says what it is and waits.
PLAIN_STANDIN = """#!/bin/sh
echo "A stand-in for $(basename "$0")"
exec sleep 600
"""
# A stand-in for Copilot CLI: it runs the hooks Cellix set up for it, as Copilot CLI runs
# them, with what Copilot CLI's docs say it sends, then asks as Copilot would.
COPILOT_STANDIN = """#!/usr/bin/env python3
import json, os, subprocess, time
print("A stand-in for Copilot CLI, running the hooks Cellix set up", flush=True)
with open(os.path.expanduser("~/.copilot/hooks/cellix.json")) as f:
    hooks = json.load(f)["hooks"]
def run(event, **fields):
    said = json.dumps({"session_id": "cp-1", "cwd": os.getcwd(), **fields})
    subprocess.run(["sh", "-c", hooks[event][0]["bash"]], input=said.encode())
run("SessionStart", source="startup")
run("UserPromptSubmit", prompt="Publish the crate")
run("Notification", notification_type="permission_prompt", message="Allow cargo publish?")
print("Allow cargo publish? (y/n)", flush=True)
time.sleep(600)
"""
# A stand-in for Kiro CLI: 3.5.0 when asked, and started, it runs the hooks Cellix set up for
# it as Kiro CLI runs a command hook, by its trigger, with the event's JSON on its input and
# the prompt in USER_PROMPT, then asks leave to run something.
KIRO_STANDIN = """#!/usr/bin/env python3
import json, os, subprocess, sys, time
if sys.argv[1:] == ["--version"]:
    print("kiro-cli 3.5.0")
    sys.exit(0)
print("A stand-in for Kiro CLI, running the hooks Cellix set up", flush=True)
with open(os.path.expanduser("~/.kiro/hooks/cellix.json")) as f:
    hooks = {hook["trigger"]: hook["action"]["command"] for hook in json.load(f)["hooks"]}
def run(event, prompt="", **fields):
    said = json.dumps({"hook_event_name": event, "cwd": os.getcwd(),
                       "session_id": "f2946a26-3735-4b08-8d05-c928010302d5", **fields})
    subprocess.run(["sh", "-c", hooks[event]], input=said.encode(),
                   env={**os.environ, "USER_PROMPT": prompt})
run("SessionStart")
run("UserPromptSubmit", prompt="Refund route for unpaid orders")
run("PermissionRequest", tool_name="execute_bash", tool_input={"command": "cargo publish"},
    permission={"capability": "shell", "command": "cargo publish"})
print("Allow cargo publish? [y/n/t]", flush=True)
time.sleep(600)
"""

RUN = """use crate::queue::{Full, Queue};

/// Loads the jobs and queues as many as fit.
pub fn run(path: &Path) -> usize {
    let mut queue = Queue::new(8);
    let jobs = load_jobs(path);
    let mut sent = 0;
    for job in jobs {
        if let Err(full) = queue.push(job) {
            log(Level::Warn, "queue is full");
            break;
        }
        sent += 1;
    }
    sent
}
"""


def code_intelligence(scratch):
    """Go to Symbol, signature help and inlay hints, from a scripted server that answers at
    once, in a project of their own so other scenes keep no server."""
    root = f"{scratch}/lang"
    os.makedirs(f"{root}/src")
    os.makedirs(f"{root}/.cellix")
    more = CODE.replace("    }\n}\n", """    }

    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_full(&self) -> bool {
        self.items.len() == self.limit
    }
}

/// Returned when the queue is full.
pub struct Full;
""")
    files = {"src/queue.rs": more, "src/run.rs": RUN, "Cargo.toml": '[package]\nname = "queue"\n',
             "src/jobs.rs": "pub fn push_job() {}\npub fn pushes_until_full() {}\npub const PUSH_LIMIT: usize = 8;\n"}
    for name, text in files.items():
        with open(f"{root}/{name}", "w") as f:
            f.write(text)
    server = {"language_servers": {"Rust": ["python3", "-I", f"{here}/fake_lsp.py"]}}
    with open(f"{root}/.cellix/settings.json", "w") as f:
        json.dump(server, f)
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "First")
    symbols = [["wait", 1.0], ["keys", "\x1b[111;6u"], ["wait", 1.0]]
    down = [["keys", "\x1b[B"]] * 5
    shoot("120-go-to-symbol", [*symbols, *down, ["wait", 0.5]], "src/queue.rs", cwd=root)
    shoot("121-go-to-symbol-typed", [*symbols, ["keys", "pu"], ["wait", 0.5]], "src/queue.rs", cwd=root)
    shoot("122-workspace-symbols", [["wait", 1.0], ["keys", CTRL("t")], ["keys", "push"], ["wait", 1.0]],
          "src/queue.rs", cwd=root)
    typing = [["wait", 1.0], ["keys", CTRL("l")], ["keys", "19\r"], ["keys", "\x1b[F"],
              ["keys", "\r"], ["keys", "self.items.insert(0, "], ["wait", 1.0]]
    shoot("123-signature-help", typing, "src/queue.rs", cwd=root)
    shoot("124-inlay-hints", [["wait", 1.5]], "src/run.rs", cwd=root)
    shoot("125-inlay-hints-light", [["wait", 1.5]], "src/run.rs", cwd=root, background="eff1f5")


TERRAFORM = """terraform {
  required_version = ">= 1.6"
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 5.0"
    }
  }
}

provider "aws" {
  region = var.region
}

module "network" {
  source = "./modules/network"
  cidr   = "10.0.0.0/16"
}

resource "aws_instance" "api" {
  ami           = data.aws_ami.ubuntu.id
  instance_type = "t3.small"
  subnet_id     = module.network.private_subnet_ids[0]
}
"""


def install_card(scratch):
    """The card in the window's corner that offers a language's server, for a Terraform file
    where terraform-ls isn't installed: the PATH here is the system's own, which has none.
    Nothing presses Install, which would run a real installer."""
    root = f"{scratch}/terraform"
    os.makedirs(root)
    files = {"main.tf": TERRAFORM, "variables.tf": 'variable "region" {\n  type = string\n}\n'}
    for name, text in files.items():
        with open(f"{root}/{name}", "w") as f:
            f.write(text)
    git(root, "init", "-q", "-b", "main")
    git(root, "add", "-A")
    git(root, "commit", "-qm", "First")
    bare = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"}
    shoot("108-install-card", [["wait", 1.0]], "main.tf", cwd=root, env=bare)
    # Not Now, under the pointer.
    shoot("108-install-card-hover", [["move", *INSTALL_NOT_NOW], ["wait", 0.4]],
          "main.tf", cwd=root, env=bare)
    shoot("108-install-card-light", [["wait", 1.0]], "main.tf", cwd=root, env=bare, background="eff1f5")
    # Over the side pane, which isn't the file's colour: the card's corners show the pane's.
    shoot("108-install-card-pane", [["wait", 1.0], *command("show comments", F1), ["wait", 1.0]],
          "main.tf", cwd=root, env=bare)
    # A language only Homebrew installs, with no Homebrew: what it needs, and its command to
    # copy in place of an Install that could only fail.
    java = f"{scratch}/java"
    os.makedirs(java)
    with open(f"{java}/Main.java", "w") as f:
        f.write("class Main {\n    public static void main(String[] args) {}\n}\n")
    git(java, "init", "-q", "-b", "main")
    shoot("108-install-card-needs", [["wait", 1.0]], "Main.java", cwd=java, env=bare)


def git_scenes(scratch):
    """Git past staging: a merge stopped on a conflict, stashes, the graph, a file's timeline
    and Compare With, in a project of its own with some history."""
    root = f"{scratch}/history"
    os.makedirs(f"{root}/src")
    def write(name, text):
        with open(f"{root}/{name}", "w") as f:
            f.write(text)
    # A minute apart, the last a minute ago, so git lists them in the order they were made:
    # made in one second, they came in the order of their branches' names, the agent's first.
    made = [int(time.time()) - 6 * 60]
    def commit(message):
        made[0] += 60
        when = f"{made[0]} +0000"
        git(root, "add", "-A")
        git(root, "commit", "-qm", message, env={"GIT_AUTHOR_DATE": when, "GIT_COMMITTER_DATE": when})
    write("src/queue.rs", CODE)
    write("README.md", "# Queue\n")
    git(root, "init", "-q", "-b", "main")
    commit("Start the queue")
    git(root, "checkout", "-q", "-b", "agent/queue-bound")
    write("src/queue.rs", CODE.replace("Result<(), Full>", "Result<(), T>").replace("Err(Full)", "Err(item)"))
    commit("Return the item when the queue is full")
    git(root, "checkout", "-q", "main")
    write("README.md", "# Queue\n\nA bounded queue.\n")
    commit("Say what it is")
    git(root, "checkout", "-q", "-b", "docs")
    write("PLAN.md", "# Plan\n\nBound the queue.\n")
    commit("Write the plan")
    git(root, "checkout", "-q", "main")
    git(root, "merge", "-q", "--no-edit", "docs")
    write("src/queue.rs", CODE.replace("self.items.len() == self.limit", "self.items.len() >= self.limit"))
    commit("Bound the queue")
    # Two stashes, one put aside with a message.
    write("README.md", "# Queue\n\nA bounded queue, for now.\n")
    git(root, "stash", "push", "-q", "-m", "Tidy the README")
    write("PLAN.md", "# Plan\n\nRing buffer?\n")
    git(root, "stash", "push", "-q")
    graph = [["wait", 1.0], *command("Show Git History"), ["wait", 3.0]]
    shoot("81-git-graph", graph, "README.md", size="120x40", cwd=root)
    # The newest commit's row, opened to its files.
    # The newest commit's row, opened to its files: the graph's heading is the panel's top row.
    # Its message shares the row with main's chip, so the screen's letters hold only "Bound t".
    shoot("82-git-commit-open", [*graph, ["wait", 3.0], ["click", *ON("Bound")], ["wait", 2.0]], "README.md", size="120x40", cwd=root)
    shoot("83-git-timeline", [["wait", 1.5]], "src/queue.rs", size="120x40", cwd=root)
    compare = [["wait", 1.0], ["keys", "\x1b[112;6u"], ["until", PALETTE_UP], ["keys", "Compare With"],
               ["until", "Compare With"], ["keys", "\r"], ["until", "Compare the file"]]
    shoot("84-git-compare", compare, "src/queue.rs", size="120x40", cwd=root)
    shoot("85-git-compared", [*compare, ["keys", "agent"], ["wait", 0.8], ["keys", "\r"], ["wait", 1.5]],
          "src/queue.rs", size="120x40", cwd=root)
    # Then a merge that stops on a conflict.
    subprocess.run(["git", "-c", "user.name=tour", "-c", "user.email=tour@example.com", "merge", "-q",
                    "agent/queue-bound"], cwd=root, capture_output=True)
    source = [["wait", 1.0], ["click", *REVIEW], ["wait", 3.0]]
    shoot("86-git-merge-panel", source, "src/queue.rs", size="120x40", cwd=root)
    shoot("87-git-conflict", [["wait", 0.8]], "src/queue.rs", size="120x40", cwd=root)
    shoot("88-git-conflict-hover", [["wait", 1.0], ["move", *ON("Incoming")], ["wait", 0.6]],
          "src/queue.rs", size="120x40", cwd=root)
    shoot("89-git-stashes", [*source, ["wheel", 14, 20, 3], ["wait", 0.4]], "src/queue.rs", size="120x40", cwd=root)


def transcript_lines(root, scale=1.0):
    """A short Claude Code session in the shape its transcript has: two turns, with reads,
    an edit, a failing test run and replies, and what each reply took in."""
    lines = [{"type": "custom-title", "customTitle": "Add a refund route"}]
    used = [0]

    def user(text):
        lines.append({"type": "user", "cwd": root, "message": {"role": "user", "content": text}})

    def reply(blocks, context):
        context = int(context * scale)
        used[0] += 1
        lines.append({"type": "assistant", "cwd": root, "message": {
            "id": f"m{used[0]}", "model": "claude-opus-5-5", "role": "assistant", "content": blocks,
            "usage": {"input_tokens": 2, "cache_creation_input_tokens": 2000,
                      "cache_read_input_tokens": context - 2002, "output_tokens": 120}}})

    def tool(id, name, args, context):
        reply([{"type": "tool_use", "id": id, "name": name, "input": args}], context)

    def result(id, text, error=False):
        lines.append({"type": "user", "cwd": root, "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": id, "content": text, "is_error": error}]}})

    user("Add a POST /orders/:id/refund route. Only full refunds for now, and a test.")
    reply([{"type": "text", "text": "I'll look at how orders are routed first, then add the route beside create."}], 41000)
    tool("t1", "Read", {"file_path": f"{root}/src/queue.rs"}, 52000)
    result("t1", CODE)
    tool("t2", "Grep", {"pattern": "fn create", "path": f"{root}/src"}, 58000)
    result("t2", "src/orders.rs:12: pub fn create(")
    tool("t3", "Edit", {"file_path": f"{root}/src/orders.rs", "old_string": "a\nb",
                        "new_string": "a\nb\nc\nd\ne"}, 71000)
    result("t3", "The file was updated.")
    tool("t4", "Bash", {"command": "cargo test orders"}, 80000)
    result("t4", "running 15 tests\n" + "".join(f"test orders::case_{n} ... ok\n" for n in range(14))
           + "test orders::refund_full ... FAILED\n\nthread 'refund_full' panicked: left: Paid, right: Refunded\n"
           "test result: FAILED. 14 passed; 1 failed", True)
    reply([{"type": "text", "text": "The refund never changes the order's state. I'll set it in the handler and run the tests again."}], 88000)
    tool("t5", "Bash", {"command": "cargo test orders"}, 96000)
    result("t5", "running 15 tests\ntest result: ok. 15 passed")
    reply([{"type": "text", "text": "Added `POST /orders/:id/refund` and `refund_full`. It sets the order to **Refunded**, and all 15 order tests pass."}], 98000)
    user("Also reject a refund on an order that was never paid.")
    tool("t6", "Edit", {"file_path": f"{root}/src/orders.rs", "old_string": "x", "new_string": "x\ny\nz"}, 112000)
    result("t6", "The file was updated.")
    reply([{"type": "text", "text": "A refund on an unpaid order now answers 409 Conflict, with a test."}], 124000)
    return "".join(json.dumps(line) + "\n" for line in lines)


def transcripts(scratch):
    """An agent in the side pane whose hooks have said where its transcript is: its tab
    with how full its context is, and the transcript as a page. The agent is a stand-in that
    reports through Cellix's real hook, as Claude Code's plugin does, and then waits."""
    tools = f"{scratch}/bin"
    os.makedirs(tools)
    log = f"{scratch}/transcript.jsonl"
    with open(log, "w") as f:
        f.write(transcript_lines(ROOT))
    # The same session nearer the end of its window: 172k of 200k.
    full = f"{scratch}/transcript-full.jsonl"
    with open(full, "w") as f:
        f.write(transcript_lines(ROOT, 172 / 124))
    said = json.dumps({"session_id": "abc-123", "cwd": ROOT, "transcript_path": "LOG"})
    said = said.replace('"LOG"', '"$LOG"')
    with open(f"{tools}/claude", "w") as f:
        f.write(f"#!/bin/sh\nprintf 'Claude Code (stand-in)\\n'\nLOG=${{TRANSCRIPT:-{log}}}\n"
                f"printf '%s' \"{said.replace(chr(34), chr(92) + chr(34))}\" | '{binary}' hook session-start\n"
                f"exec sleep 300\n")
    os.chmod(f"{tools}/claude", 0o755)
    # The meter reads the transcript every couple of seconds, away from the screen.
    start = [*command("Open Claude Code", F1), ["until", "stand-in"], ["until", "%"]]
    opened = [*start, *command("Agent Transcript", F1), ["until", "Show Source"], ["wait", 1.0]]
    scenes = [
        ("64-transcript-tab", start, [CODE_FILE], {}),
        # A tab is as wide as what it paints, so its cells hold the start of its name: the
        # conversation's title its transcript keeps, its terminal's being only "Claude Code".
        ("64-transcript-tab-hover", [*start, ["move", *ON("Add a ref")], ["wait", 0.6]], [CODE_FILE], {}),
        ("64-transcript-tab-menu", [*start, ["right", *ON("Add a ref")], ["wait", 0.4]], [CODE_FILE], {}),
        ("64-transcript-tab-full", start, [CODE_FILE], {"TRANSCRIPT": full}),
        ("64-transcript-tab-light", start, [CODE_FILE], {"background": "eff1f5"}),
        ("65-transcript-page", opened, [], {}),
        ("66-transcript-light", opened, [], {"background": "eff1f5"}),
        ("67-transcript-scrolled", [*opened, ["wheel", 50, 20, 6], ["wait", 0.6]], [], {}),
    ]
    # Each scene has sessions of its own, so none finds the last one's, running or stopped.
    for name, steps, files, extra in scenes:
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        if "TRANSCRIPT" in extra:
            env["TRANSCRIPT"] = extra.pop("TRANSCRIPT")
        shoot(name, steps, *files, size="150x44", env=env, **extra)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
        shutil.rmtree(f"{ROOT}/.cellix/transcripts", ignore_errors=True)
def agent_tabs(scratch):
    """Agents in the side pane, each tab with its program's mark, the name its own title gives
    it, and what it's doing: working, asking, done while behind another tab, or nothing. The
    agents are stand-ins that name themselves in their terminal's title as Claude Code does;
    what they're doing comes through Cellix's real hook."""
    tools = f"{scratch}/agent-bin"
    os.makedirs(tools)
    for program, title in (("claude", "✳ pricing page"), ("codex", "og tags"), ("gemini", "docs nav")):
        with open(f"{tools}/{program}", "w") as f:
            f.write(f"#!/bin/sh\nprintf '\\033]0;{title}\\007'\nprintf '{program} (stand-in)\\n'\n"
                    "exec sleep 300\n")
        os.chmod(f"{tools}/{program}", 0o755)

    def hook(kind, agent="Claude Code", **fields):
        stdin = json.dumps({"session_id": f"s-{agent}", "cwd": ROOT, **fields})
        named = "" if agent == "Claude Code" else f"--agent {agent} "
        return ["run", f"'{binary}' hook {named}{kind}", stdin]

    working = lambda agent="Claude Code": hook("tool-done", agent, tool_input={"command": "ls"})
    asking = lambda agent="Claude Code": hook("notify", agent, notification_type="permission_prompt",
                                              message="Claude needs your permission to use Bash")
    # A narrow pane cuts the name in front short too, so only its first word is waited for.
    start = [*command("Open Claude Code", F1), ["until", "pricing"], ["wait", 0.4]]
    # Codex last, so it's in front and Gemini CLI finishes behind it. In the narrow window its
    # name is cut short while the button that maximizes the pane has room ("og t…").
    others = [*command("Open Gemini CLI", F1), ["until", "docs nav"], *command("Open Codex", F1),
              ["until", "og t"], ["wait", 0.4]]
    behind = [*start, *command("New Terminal", F1), ["wait", 0.8]]
    scenes = [
        ("m1-agent-tab", start, {}),
        ("m1-agent-tab-working", [*start, working(), ["wait", 0.8]], {}),
        ("m1-agent-tab-asking", [*start, asking(), ["wait", 0.8]], {}),
        ("m1-agent-tab-done", [*behind, working(), hook("stop"), ["wait", 0.8]], {}),
        ("m1-agent-tab-menu", [*start, ["right", *ON("pricing")], ["wait", 0.4]], {}),
        ("m1-agent-tab-renamed", [*start, ["right", *ON("pricing")], ["wait", 0.4],
                                   ["click", *ON("Rename")], ["wait", 0.3], ["keys", "\x7f" * 12],
                                   ["keys", "billing\r"], ["until", "bill"], ["wait", 0.3]], {}),
        # Rename tab's box with its caret moved back a word by Option+Left, as a terminal sends
        # it, into the middle of the name: before "page".
        ("m1-agent-tab-rename-caret", [*start, ["right", *ON("pricing")], ["wait", 0.4],
                                        ["click", *ON("Rename")], ["until", "Rename tab"],
                                        ["keys", "\x1bb"], ["wait", 0.4]], {}),
        ("m1-agent-tabs", [*start, working(), *others, asking("Codex"), working("Gemini"),
                            hook("stop", "Gemini"), ["wait", 0.8]], {}),
        ("m1-agent-tabs-narrow", [*start, working(), *others, asking("Codex"), working("Gemini"),
                                   hook("stop", "Gemini"), ["wait", 0.8]], {"size": "96x40"}),
        ("m1-agent-tabs-cells", [*start, working(), *others, asking("Codex"), working("Gemini"),
                                  hook("stop", "Gemini"), ["wait", 0.8]], {"CELLIX_TIER": "cells"}),
        # The same run as the dark one: Gemini CLI done behind the others.
        ("m1-agent-tabs-light", [*start, working(), *others, asking("Codex"), working("Gemini"),
                                  hook("stop", "Gemini"), ["wait", 0.8]], {"background": "eff1f5"}),
    ]
    for name, steps, extra in scenes:
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        size = extra.pop("size", "150x44")
        background = extra.pop("background", DARK)
        env.update(extra)
        shoot(name, steps, CODE_FILE, size=size, env=env, background=background)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)


def agent_names(scratch):
    """Three Claude Codes whose titles say only "Claude Code": the first goes by its
    conversation's title, which its made-up transcript keeps, the second by the first thing
    asked of it, cut short, and the third, asked much the same, by that and " (2)". Each is a
    stand-in that reports its prompt through Cellix's real hook, from its own process, so
    Cellix takes it for its tab."""
    tools = f"{scratch}/names-bin"
    os.makedirs(tools)
    log = f"{tools}/titled.jsonl"
    with open(log, "w") as f:
        f.write(json.dumps({"type": "ai-title", "aiTitle": "Refund route for unpaid orders",
                            "sessionId": "names-1"}) + "\n")
    asked = ["Add a refund route for unpaid orders",
             "Fix the login redirect after a password reset, and add a test for it",
             "Fix the login redirect after a password change"]
    for n, prompt in enumerate(asked, 1):
        said = {"session_id": f"names-{n}", "cwd": ROOT, "prompt": prompt}
        if n == 1:
            said["transcript_path"] = log
        with open(f"{tools}/prompt-{n}.json", "w") as f:
            f.write(json.dumps(said))
    # Each start takes the next prompt, by a count kept beside them.
    with open(f"{tools}/claude", "w") as f:
        f.write("#!/bin/sh\nprintf '\\033]0;\\342\\234\\263 Claude Code\\007'\n"
                "printf 'Claude Code (stand-in)\\n'\n"
                f"n=$(( $(cat '{tools}/count' 2>/dev/null || echo 0) + 1 )); echo $n > '{tools}/count'\n"
                f"sh -c \"'{binary}' hook prompt < '{tools}/prompt-$n.json'\"\n"
                "exec sleep 300\n")
    os.chmod(f"{tools}/claude", 0o755)
    # A retake starts the count again, so each start has its own prompt again.
    # The cells hold as much of a tab's name as its columns do, so the third's " (2)", painted
    # after its words, isn't among the screen's words: it's given a moment instead.
    opened = [["run", f"rm -f '{tools}/count'", ""]]
    for words in ("Refund route", "Fix the login", "Fix the login"):
        opened += [*command("Open Claude Code", F1), ["until", words], ["wait", 1.0]]
    scenes = [
        ("m1-agent-names", opened, {}),
        # The one in front, cut short, has what was asked whole in its tip.
        ("m1-agent-names-tip", [*opened, ["move", *ON("Fix the login", 3)], ["wait", 0.8]], {}),
        ("m1-agent-names-list", [*opened, *command("List Agents", F1), ["until", "WORKING"],
                                 ["wait", 0.6]], {}),
        ("m1-agent-names-light", opened, {"background": "eff1f5"}),
    ]
    for name, steps, extra in scenes:
        if os.path.exists(f"{tools}/count"):
            os.unlink(f"{tools}/count")
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        background = extra.pop("background", DARK)
        shoot(name, steps, CODE_FILE, size="150x44", env=env, background=background)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)


def agent_in_terminal(scratch):
    """`claude` typed at a Terminal tab's prompt: the tab is named and marked as an agent's
    tab is, by what Claude Code says of its process. The stand-in writes what Claude Code
    writes, the file for its process (`sessions/<pid>.json`) and its conversation's transcript,
    titled, in the stand-in home; titles its terminal as Claude Code does before it has a title;
    says it has finished through Cellix's real hook, run through a shell from its own process
    as Claude Code runs it; and waits, as `sleep`, which isn't Claude Code's name."""
    tools = f"{scratch}/terminal-bin"
    os.makedirs(tools)
    lines = [{"type": "user", "message": {"role": "user", "content": "Add a refund route for unpaid orders"}},
             {"type": "ai-title", "aiTitle": "Refund route for unpaid orders", "sessionId": "term-1"}]
    with open(f"{tools}/term-1.jsonl", "w") as f:
        f.write("".join(json.dumps(line) + "\n" for line in lines))
    with open(f"{tools}/stop.json", "w") as f:
        f.write(json.dumps({"session_id": "term-1", "cwd": ROOT, "hook_event_name": "Stop"}))
    record = ('{"pid":%s,"sessionId":"term-1","cwd":"%s","kind":"interactive","entrypoint":"cli",'
              '"name":"queue","nameSource":"derived","status":"idle"}')
    with open(f"{tools}/claude", "w") as f:
        f.write("#!/bin/sh\n"
                'home="${CLAUDE_CONFIG_DIR:-$HOME/.claude}"\n'
                "folder=$(printf '%s' \"$PWD\" | tr -c 'a-zA-Z0-9' '-')\n"
                'mkdir -p "$home/sessions" "$home/projects/$folder"\n'
                f"cp '{tools}/term-1.jsonl' \"$home/projects/$folder/term-1.jsonl\"\n"
                f"printf '{record}' $$ \"$PWD\" > \"$home/sessions/$$.json\"\n"
                "printf '\\033]0;\\342\\234\\263 Claude Code\\007'\n"
                "printf 'Claude Code (stand-in)\\n'\n"
                # A list, as Claude Code's hook command is, so the shell runs the hook as a
                # child and the hook finds the agent as that shell's parent.
                f"sh -c \"'{binary}' hook stop < '{tools}/stop.json' || true\"\n"
                "exec sleep 300\n")
    os.chmod(f"{tools}/claude", 0o755)
    typed = [*command("New Terminal", F1), ["wait", 1.5], ["keys", "claude\r"], ["until", "stand-in"],
             ["until", "Refund route"], ["wait", 0.6]]
    scenes = [
        ("m1-agent-in-terminal", {}),
        ("m1-agent-in-terminal-light", {"background": "eff1f5"}),
        ("m1-agent-in-terminal-cells", {"CELLIX_TIER": "cells"}),
    ]
    folder = "".join(c if c.isascii() and c.isalnum() else "-" for c in ROOT)
    for name, extra in scenes:
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        background = extra.pop("background", DARK)
        env.update(extra)
        shoot(name, typed, CODE_FILE, size="150x44", env=env, background=background)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
        # What the stand-in wrote as Claude Code's goes with it.
        shutil.rmtree(f"{HOME}/.claude/sessions", ignore_errors=True)
        shutil.rmtree(f"{HOME}/.claude/projects/{folder}", ignore_errors=True)


PROMPT_STAND_IN = r'''import os, sys, tty
# Claude Code's prompt, as far as the keyboard goes: it asks to hear of focus (DECSET 1004),
# hides the terminal's cursor and draws its own, a block, only while told it has the keys.
tty.setraw(0)
focused, typed, waiting = True, "", b""
def draw():
    width = max(20, min(os.get_terminal_size(1).columns - 2, 60))
    cursor = "\x1b[7m \x1b[27m" if focused else " "
    rule = "\x1b[90m" + "─" * width + "\x1b[39m"
    sys.stdout.write(f"\x1b[?25l\x1b[3;1H\x1b[2K{rule}\x1b[4;1H\x1b[2K> {typed}{cursor}"
                     f"\x1b[5;1H\x1b[2K{rule}\x1b[6;1H\x1b[2K\x1b[90m  ? for shortcuts\x1b[39m")
    sys.stdout.flush()
sys.stdout.write("\x1b[?1004h\x1b[2J\x1b[1;1HClaude Code (stand-in)")
draw()
while True:
    came = os.read(0, 1024)
    if not came:
        break
    waiting += came
    while waiting:
        if waiting.startswith(b"\x1b[I") or waiting.startswith(b"\x1b[O"):
            focused, waiting = waiting[2:3] == b"I", waiting[3:]
        elif waiting[:1] == b"\x1b":
            if len(waiting) < 3:
                break
            waiting = waiting[1:]
        else:
            letter, waiting = waiting[:1].decode(errors="ignore"), waiting[1:]
            typed += letter if letter.isprintable() else ""
    draw()
'''


def prompt_keys(scratch):
    """Where the keyboard is, as Claude Code's prompt shows it: its cursor is there only while
    the keys are. The agent is a stand-in that asks to hear of focus and draws its own block
    only while told it has the keys, as Claude Code does. With the keys in the file it has none;
    pressed, it has them and what's typed goes there; with the window behind another, none. Its
    workspace opened again, with its tab in front, starts with the keys in it."""
    tools = f"{scratch}/prompt-bin"
    os.makedirs(tools)
    with open(f"{tools}/prompt.py", "w") as f:
        f.write(PROMPT_STAND_IN)
    with open(f"{tools}/claude", "w") as f:
        f.write(f"#!/bin/sh\nexec python3 -I '{tools}/prompt.py'\n")
    os.chmod(f"{tools}/claude", 0o755)
    opened = [*command("Open Claude Code", F1), ["until", "for shortcuts"], ["wait", 0.4]]
    in_file = [*opened, ["click", *ON("pub struct")], ["wait", 0.8]]
    typed = [*in_file, ["click", *ON("for shortcuts")], ["keys", "fix the queue"],
             ["until", "fix the queue"], ["wait", 0.6]]
    # The window goes behind another: the terminal says so (focus reporting's CSI O).
    behind = [*opened, ["keys", "\\e[O"], ["wait", 0.8]]
    cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
    env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
    shoot("m1-prompt-keys-in-file", in_file, CODE_FILE, size="150x44", env=env,
          setup_for=("m1-prompt-keys-reopened",))
    # The same project opened again, with nothing named: the agent kept running, its tab in
    # front, and the keyboard starts in it, so the first keys go there.
    again = [["until", "for shortcuts"], ["wait", 0.8], ["keys", "go on"], ["until", "go on"],
             ["wait", 0.6]]
    shoot("m1-prompt-keys-reopened", again, size="150x44", env=env)
    subprocess.run(["pkill", "-9", "-f", cache])
    shutil.rmtree(cache, ignore_errors=True)
    for name, steps in (("m1-prompt-keys-typed", typed), ("m1-prompt-keys-window-behind", behind)):
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        shoot(name, steps, CODE_FILE, size="150x44", env=env)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)


def team_scenes(scratch):
    """An agent starting another: the card that asks the person first, and the helper once
    it's allowed. Both agents are stand-ins on PATH. Claude Code's starts Codex's through the
    real `cellix agent run`; Codex's says it's ready through Cellix's real hook, takes its task
    and prints what it was given."""
    tools = f"{scratch}/team-bin"
    os.makedirs(tools)
    task = "Review the changes in src/queue.rs. Leave each finding with cellix suggest."
    # Each runs its hook through a shell, as an agent does, so the hook's report is the
    # stand-in's own and Cellix takes it for its session.
    def hook(kind, said, agent=""):
        named = f" --agent {agent}" if agent else ""
        said_file = f"{tools}/{kind}-{agent or 'claude'}.json"
        with open(said_file, "w") as f:
            f.write(json.dumps(said))
        return f"sh -c \"'{binary}' hook{named} {kind} < '{said_file}'\"\n"
    lead = {"session_id": "lead-1", "cwd": ROOT}
    with open(f"{tools}/claude", "w") as f:
        f.write("#!/bin/sh\nprintf 'Claude Code (stand-in)\\n'\n"
                "printf 'I will hand the review to a helper.\\n'\n"
                + hook("prompt", {**lead, "prompt": "Review the queue"}) +
                f"'{binary}' agent run --name reviewer codex '{task}' >/dev/null 2>&1\n"
                "printf 'Asked reviewer to look over the queue.\\n'\n"
                "exec sleep 300\n")
    with open(f"{tools}/codex", "w") as f:
        f.write("#!/bin/sh\nprintf 'Codex (stand-in)\\n'\n"
                + hook("session-start", {"session_id": "rev-1", "cwd": ROOT}, "Codex") +
                "while IFS= read -r line; do printf 'got: %s\\n' \"$line\"; done\n")
    for name in ("claude", "codex"):
        os.chmod(f"{tools}/{name}", 0o755)
    asks = [*command("Open Claude Code", F1), ["until", "stand-in"], ["until", "wants to start"],
            ["wait", 0.6]]
    # Allow, the last of the card's buttons, by its own words: 27 columns past Deny landed on
    # Always Allow Here in a narrow pane, and the consent it kept was never asked for again.
    # Then the lead says so once the helper has started: in a narrow pane the helper's tab is
    # its mark alone, and "Codex" was waited for there in vain.
    allowed = [*asks, ["click", *ON("Allow", 1, 0, -1)], ["gone", "wants to start"],
               ["until", "Asked reviewer"], ["wait", 1.5]]
    scenes = [
        ("170-agent-asks", asks, {}),
        ("170-agent-asks-light", asks, {"background": "eff1f5"}),
        ("170-agent-asks-narrow", asks, {"size": "100x40"}),
        ("171-agent-started", allowed, {}),
        ("172-agents-helper", [*allowed, *LIST_AGENTS, ["until", "reviewer"], ["wait", 0.6]], {}),
        ("173-peek", [*allowed, *LIST_AGENTS, ["until", "reviewer"], ["click", *ON("reviewer")], ["until", "Keep as Tab"], ["wait", 0.8]], {}),
        ("173-peek-light", [*allowed, *LIST_AGENTS, ["until", "reviewer"], ["click", *ON("reviewer")], ["until", "Keep as Tab"], ["wait", 0.8]], {"background": "eff1f5"}),
        ("173-peek-narrow", [*allowed, *LIST_AGENTS, ["until", "reviewer"], ["click", *ON("reviewer")], ["until", "Keep as Tab"], ["wait", 0.8]], {"size": "100x40"}),
        # Ctrl+click on the helper's name in the lead's own words opens it the same way.
        ("174-name-peek", [*allowed, ["until", "Asked reviewer"], ["ctrl", *ON("Asked reviewer", 7)], ["until", "Keep as Tab"], ["wait", 0.8]], {}),
    ]
    # Each with a cache of its own, which a retake makes again: what's kept there, the
    # folder's consent among it, is never a later try's.
    for name, steps, extra in scenes:
        env = {"PATH": f"{tools}:{os.environ['PATH']}"}
        if "CELLIX_TIER" in extra:
            env["CELLIX_TIER"] = extra.pop("CELLIX_TIER")
        shoot(name, steps, CODE_FILE, **{"size": "150x44", **extra}, env=env)


def reviewer_scenes(scratch):
    """Review with an agent: the menu from Review's list, and what the agent suggests under
    its line. Codex is a stand-in on PATH that leaves one finding through the real
    `cellix suggest` as it starts, as the Reviewer role tells it to."""
    tools = f"{scratch}/review-bin"
    os.makedirs(tools)
    with open(f"{tools}/codex", "w") as f:
        f.write("#!/bin/sh\nprintf 'Codex (stand-in)\\n'\n"
                f"'{binary}' suggest src/queue.rs 24 Clarity '/// Returned when the queue is full.' "
                "'Say what a caller should do when it gets this.' >/dev/null 2>&1\n"
                "printf 'Left 1 finding.\\n'\n"
                "exec sleep 300\n")
    os.chmod(f"{tools}/codex", 0o755)
    # The menu lists the agents installed, here the stand-in Codex, and ends with the row that
    # sets up more.
    menu = [["keys", "\u0007"], ["until", "Review with"], ["click", *ON("Review with")],
            ["until", "Set up more"], ["wait", 0.4]]
    found = [*menu, ["click", *ON("Codex")], ["until", "Suggested by Codex"], ["wait", 1.0]]
    scenes = [
        ("175-review-with", menu, {}),
        ("176-review-suggested", found, {}),
        ("176-review-suggested-light", found, {"background": "eff1f5"}),
    ]
    threads = f"{ROOT}/.cellix/threads.json"
    for name, steps, extra in scenes:
        # Each starts with no finding: the light one began with the dark one's, so it saw
        # "Suggested by Codex" at once and was pictured before its Codex had started.
        if os.path.exists(threads):
            os.unlink(threads)
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        shoot(name, steps, CODE_FILE, **{"size": "150x44", **extra}, env=env)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
    # What the finding leaves in the file: its line's ◆ in the gutter. Then the finding goes,
    # or every later scene of this file has it: 02-code showed a ◆ that 0.10.2 didn't.
    # It writes the finding itself, as `cellix suggest` leaves it, so it shows alone too.
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    finding = {"id": 1, "file": CODE_FILE, "line": 24, "quote": "/// Returned when the queue is full.",
               "text": "Say what a caller should do when it gets this.", "replies": [],
               "resolved": False, "told": 0, "asked": False, "kind": "Clarity",
               "suggested_by": "Codex"}
    with open(threads, "w") as f:
        json.dump([finding], f)
    shoot("177-code-comment-mark", [["wait", 1.0]], CODE_FILE, cellix=True)
    if os.path.exists(threads):
        os.unlink(threads)


def ask_scenes(scratch):
    """Ask, a side question about a selection: the field under the words, an answer streaming
    in, the answer whole with its buttons, a follow-up being typed, and Ask with no agent
    installed. Claude Code is a stand-in on a PATH of its own that prints what the real one
    prints for a made-up answer: no real agent runs, and nothing here is anyone's words."""
    tools = f"{scratch}/ask-bin"
    slow = f"{scratch}/ask-slow-bin"
    for folder in (tools, slow):
        os.makedirs(folder)
    answer = ("Softmax turns the similarity scores into weights: it raises e to each score and "
              "divides by their sum, so every weight is positive and they add up to one. Here it "
              "makes each output a weighted average of the inputs, the larger scores counting more.")
    init = json.dumps({"type": "system", "subtype": "init", "session_id": "side-tour"})
    def delta(text):
        return json.dumps({"type": "stream_event", "event": {"type": "content_block_delta",
                                                            "delta": {"type": "text_delta", "text": text}}})
    first, rest = answer[:62], answer[62:]
    whole = [init, delta(first), delta(rest),
             json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": answer})]
    # The streaming one stops after its first words, as a run still thinking does.
    for folder, lines, tail in ((tools, whole, ""), (slow, [init, delta(first)], "exec sleep 300\n")):
        with open(f"{folder}/lines", "w") as f:
            f.write("\n".join(lines) + "\n")
        with open(f"{folder}/claude", "w") as f:
            f.write(f"#!/bin/sh\ncat > /dev/null\ncat '{folder}/lines'\n{tail}")
        os.chmod(f"{folder}/claude", 0o755)
    # Only the stand-in and the system's own tools: never a real agent's command line.
    system = "/usr/bin:/bin:/usr/sbin:/sbin"
    field = [["wait", 1.5], ["double", *ON("roma sfm", 6)], ["wait", 0.5], ["click", *ON("Ask (Alt")],
             ["until", "Simplify"], ["wait", 0.4]]
    asked = [*field, ["keys", "What does it do here?\r"]]
    done = [*asked, ["until", "Make a Comment"], ["wait", 0.6]]
    follow = [*done, ["click", *ON("Follow Up")], ["until", "Ask a follow-up"],
              ["keys", "And in one line?"], ["wait", 0.6]]
    # A question longer than the field is wide wraps under one box, and the caret's line shows.
    long_question = ("Why a softmax and not a plain normalisation by the sum, and would a "
                     "temperature change what the weights mean for the outputs here?")
    long = [*field, ["keys", long_question], ["wait", 0.6]]
    scenes = [
        ("q1-ask-field", field, tools, {}),
        ("q1-ask-field-long", long, tools, {}),
        ("q1-ask-field-light", field, tools, {"background": "eff1f5"}),
        ("q2-ask-streaming", [*asked, ["until", "Softmax turns"], ["wait", 0.8]], slow, {}),
        ("q3-ask-answer", done, tools, {}),
        ("q3-ask-answer-light", done, tools, {"background": "eff1f5"}),
        ("q3-ask-answer-narrow", done, tools, {"size": "100x44"}),
        ("q4-ask-follow-up", follow, tools, {}),
        ("q5-ask-no-agent", [*field[:-2], ["until", "Ask needs"], ["wait", 0.4]], None, {}),
    ]
    for name, steps, folder, extra in scenes:
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        path = f"{folder}:{system}" if folder else system
        env = {"XDG_CACHE_HOME": cache, "PATH": path}
        shoot(name, steps, NOTE_FILE, **{"size": "150x44", **extra}, env=env)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)


REMOTE_DOC = """# Queue service launch

## Traffic and capacity

At launch the service takes the order events now sent to the batch pipeline. We expect three \
times last year's peak in the first holiday week, about 42,000 events a second, and size the \
fleet for that with a third to spare.

Each partition is drained by one consumer, so a slow consumer holds back every event behind \
it. We cap a batch at 500 events and shed to the retry queue past two seconds of lag.

Capacity is reviewed weekly for the first month, then monthly, against the dashboards in the \
appendix.

## Rollout

Traffic moves in four steps of a quarter each, a day apart, with the batch pipeline kept warm \
until the last step has run a full week without a page.
"""


def forget_remote_pick():
    """Takes the remote docs' server out of the throwaway home's settings, if a scene kept one."""
    settings = f"{HOME}/.config/cellix/settings.json"
    try:
        with open(settings) as f:
            kept = json.load(f)
    except (OSError, ValueError):
        return
    if kept.pop("remote_docs_mcp", None) is not None:
        with open(settings, "w") as f:
            json.dump(kept, f)


def remote_scenes(scratch):
    """Reviewing a remote doc: the pick of the MCP server, the doc as a page with its strip and
    someone else's comment, a draft with Polish's suggestion, a card posting, one posted and one
    whose post failed. Claude Code is a stand-in on a PATH of its own that answers each kind of
    run by what it's told; Claude Code's config, in the throwaway home, names made-up servers.
    Nothing here is a real doc tool, person or doc."""
    now = int(time.time())
    link = "https://docs.example/d/queue-launch"
    read = json.dumps({
        "title": "Queue service launch", "link": link, "version": 12, "markdown": REMOTE_DOC,
        "comments": [{"id": "c3", "author": "Priya", "quote": "three times last year's peak",
                      "text": "Where does 3x come from?", "at": now - 7200, "resolved": False,
                      "replies": [{"id": "r1", "author": "Sam", "at": now - 3600,
                                   "text": "Last year's growth on the same week, rounded up."}]}]})
    polished = ("One slow consumer stalls its partition. How is that detected, and does the "
                "two-second shed cover it?")
    def result(answer):
        return "\n".join([
            json.dumps({"type": "system", "subtype": "init", "session_id": "remote-tour"}),
            json.dumps({"type": "result", "subtype": "success", "is_error": False, "result": answer}),
        ]) + "\n"
    # Each folder's stand-in answers a read and Polish alike; they differ in how a post goes.
    kinds = {"ok": result(json.dumps({"id": "c9"})), "hang": None,
             "fail": result(json.dumps({"error": "Your session has expired. Please sign in again."}))}
    tools = {}
    for kind, post in kinds.items():
        folder = f"{scratch}/remote-{kind}-bin"
        os.makedirs(folder)
        for name, text in (("read", result(read)), ("polish", result(polished)), ("post", post or "")):
            with open(f"{folder}/{name}", "w") as f:
                f.write(text)
        posting = "exec sleep 300" if post is None else f"cat '{folder}/post'"
        with open(f"{folder}/claude", "w") as f:
            f.write("#!/bin/sh\ninput=$(cat)\ncase \"$input\" in\n"
                    f"  *'read the document'*) cat '{folder}/read' ;;\n"
                    f"  *'Make this review comment'*) cat '{folder}/polish' ;;\n"
                    f"  *) {posting} ;;\nesac\n")
        os.chmod(f"{folder}/claude", 0o755)
        tools[kind] = folder
    config = f"{HOME}/.claude.json"
    with open(config, "w") as f:
        json.dump({"mcpServers": {"team-docs": {"command": "docs-mcp"}, "tracker": {"command": "tracker-mcp"}}}, f)
    system = "/usr/bin:/bin:/usr/sbin:/sbin"
    # The doc as a read leaves it, with three comments: Priya's with Sam's reply, the
    # reviewer's draft and the reviewer's posted one.
    def seed(state, extra=None, more=()):
        folder = f"{ROOT}/.cellix/remote/queue-launch"
        os.makedirs(folder, exist_ok=True)
        with open(f"{folder}/doc.md", "w") as f:
            f.write(REMOTE_DOC)
        with open(f"{folder}/remote.json", "w") as f:
            json.dump({"title": "Queue service launch", "link": link, "version": "12",
                       "server": "team-docs", "synced": now - 120, "others": 1}, f)
        with open(f"{ROOT}/.cellix/remote/.gitignore", "w") as f:
            f.write("*\n")
        doc = ".cellix/remote/queue-launch/doc.md"
        posted = {"is": "posted", "at": now - 300}
        threads = [
            {"id": 3, "file": doc, "line": 5, "quote": "three times last year's peak",
             "text": "Where does 3x come from?", "author": "Priya", "at": now - 7200,
             "told": 1, "placed": True, "remote": {"id": "c3", "state": {"is": "posted", "at": now - 7200}},
             "replies": [{"author": "Sam", "text": "Last year's growth on the same week, rounded up.",
                          "at": now - 3600, "remote": {"id": "r1", "state": {"is": "posted", "at": now - 3600}}}]},
            {"id": 4, "file": doc, "line": 7, "kind": "Correctness", "placed": True, "at": now - 600,
             "quote": "Each partition is drained by one consumer, so a slow consumer holds back every event behind it.",
             "text": "I think this is a problem because if one consumer gets slow for whatever reason "
                     "then all of the events behind it are going to wait, and the doc doesn't say what "
                     "happens then or how we'd notice.",
             "remote": {"state": state}, **(extra or {})},
            {"id": 5, "file": doc, "line": 5, "kind": "Clarity", "placed": True, "at": now - 900,
             "quote": "with a third to spare",
             "text": "A third over 42k, or over the 3x estimate?",
             "remote": {"id": "c7", "state": posted}},
        ]
        threads.extend(more)
        with open(f"{ROOT}/.cellix/threads.json", "w") as f:
            json.dump(threads, f)
    comments = [["until", "From team-docs"], *command("Show Comments"), ["until", "Others: 1"], ["wait", 0.8]]
    first = [*command("Review Remote Doc"), ["until", "Link to the doc"], ["keys", link + "\r"],
             ["until", "tracker"], ["wait", 0.6]]
    read_it = [*first, ["click", *ON("team-docs")], ["until", "From team-docs"], ["wait", 1.2]]
    scenes = [
        ("r0-remote-pick", first, "ok", None),
        ("r1-remote-page", read_it, "ok", None),
        ("r1-remote-page-light", read_it, "ok", None),
        ("r2-remote-polish", [*comments, ["click", *ON("Polish")], ["until", "Use This"], ["wait", 0.8]],
         "ok", {"is": "draft"}),
        ("r3-remote-posting", [*comments, ["click", *ON("Post to Doc")], ["until", "Posting to the doc"],
                               ["wait", 0.8]], "hang", {"is": "draft"}),
        ("r4-remote-posted", comments, "ok", {"is": "posted", "at": now - 60}),
        ("r5-remote-failed", comments, "fail",
         {"is": "failed", "why": "The doc tool asked to sign in again. Sign in (your security key), then Retry."}),
        ("r6-remote-validated", comments, "ok", {"is": "draft"}),
        # Validate All under way, its runs held, two at a time: "Validating 2 of 5" and Stop.
        ("r7-remote-validating", [*comments, ["click", *ON("Validate A")], ["until", "Validating"],
                                  ["wait", 0.8]], "hang", {"is": "draft"}),
        ("r7-remote-validating-light", [*comments, ["click", *ON("Validate A")],
                                        ["until", "Validating"], ["wait", 0.8]], "hang", {"is": "draft"}),
        # Post All Drafts asks first.
        ("r8-remote-post-all", [*comments, ["click", *ON("Post All")],
                                ["until", "comments to the doc?"], ["wait", 0.6]], "ok", {"is": "draft"}),
        ("r8-remote-post-all-light", [*comments, ["click", *ON("Post All")],
                                      ["until", "comments to the doc?"], ["wait", 0.6]], "ok", {"is": "draft"}),
    ]
    doc_file = ".cellix/remote/queue-launch/doc.md"
    more_drafts = [
        {"id": 6 + n, "file": doc_file, "line": 5, "placed": True, "at": now - 400 + n,
         "quote": quote, "text": text, "remote": {"state": {"is": "draft"}}}
        for n, (quote, text) in enumerate([
            ("first holiday week", "Which week is the first holiday week?"),
            ("order events", "All order events, or only new ones?"),
            ("retry queue", "How big can the retry queue get?"),
        ])]
    mores = {name: more_drafts for name in
             ("r7-remote-validating", "r8-remote-post-all",
              "r7-remote-validating-light", "r8-remote-post-all-light")}
    # The draft with what Validate said of it and Polish's words, kept with it.
    draft_words = ("I think this is a problem because if one consumer gets slow for whatever reason "
                   "then all of the events behind it are going to wait, and the doc doesn't say what "
                   "happens then or how we'd notice.")
    extras = {"r6-remote-validated": {
        "validation": {"verdict": "holds_up", "kind": "Correctness", "by": "Claude Code",
                       "on": draft_words,
                       "why": "Nothing in the doc says how a stalled partition is noticed or shed."},
        "polished": {"text": polished, "by": "Claude Code", "from": 38, "on": draft_words}}}
    for name, steps, kind, state in scenes:
        shutil.rmtree(f"{ROOT}/.cellix", ignore_errors=True)
        if state is not None:
            seed(state, extras.get(name), mores.get(name, ()))
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools[kind]}:{system}"}
        extra = {"background": "eff1f5"} if name.endswith("-light") else {}
        shoot(name, steps, size="170x46", env=env, cellix=state is not None, **extra)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
        # The server picked is kept machine-wide, in the throwaway home: each scene asks anew.
        forget_remote_pick()
    shutil.rmtree(f"{ROOT}/.cellix", ignore_errors=True)
    os.unlink(config)


def agents_by_urgency(scratch):
    """The Agents list by urgency, from agents in the side pane that report through Cellix's
    real hook: one asking, one done and not yet looked at, one working, one idle. The list
    shows only agents with a tab here, so each is a stand-in in a tab of its own that names
    itself in its title and gives its hook its session as it starts, from inside its tab, as
    Claude Code's plugin does. What each does next comes through the hook from outside."""
    tools = f"{scratch}/urgency-bin"
    os.makedirs(tools)
    # The first Claude Code started works and the second stays idle, counted in the scene's
    # own cache.
    for program, named, runs in (
        ("claude", "", [("s-work", "pricing page"), ("s-idle", "release notes")]),
        ("gemini", "--agent Gemini ", [("s-done", "docs nav")]),
        ("codex", "--agent Codex ", [("s-ask", "og tags")]),
    ):
        cases = "".join(f"  {n}) sid={sid}; title='{title}';;\n"
                        for n, (sid, title) in enumerate(runs, 1))
        said = '{"session_id":"%s","cwd":"' + ROOT + '"}'
        with open(f"{tools}/{program}", "w") as f:
            f.write("#!/bin/sh\n"
                    f"count=\"$XDG_CACHE_HOME/{program}-count\"\n"
                    "n=$(cat \"$count\" 2>/dev/null || echo 0); n=$((n + 1)); echo $n > \"$count\"\n"
                    f"case $n in\n{cases}esac\n"
                    "printf '\\033]0;%s\\007' \"$title\"\n"
                    f"printf '{said}' \"$sid\" | '{binary}' hook {named}session-start\n"
                    "exec sleep 300\n")
        os.chmod(f"{tools}/{program}", 0o755)

    # What each does next, each hook its own process, which stays: a hook's news is the latest
    # of its process, and one whose process has gone is let go.
    made = [0]

    def hook(kind, session, agent="Claude Code", **fields):
        made[0] += 1
        script = f"{scratch}/hook-{made[0]}.py"
        named = [] if agent == "Claude Code" else ["--agent", agent]
        stdin = json.dumps({"session_id": session, "cwd": ROOT, **fields})
        with open(script, "w") as f:
            f.write(f"import subprocess, time\nsubprocess.run({[binary, 'hook', *named, kind]!r}, "
                    f"input={stdin!r}, text=True, capture_output=True)\ntime.sleep(90)\n")
        return ["run", f"python3 -I '{script}' >/dev/null 2>&1 &"]

    told = [
        ["wait", 1.0],
        *command("Open Claude Code", F1), ["until", "pricing"],
        *command("Open Claude Code", F1), ["until", "release"],
        *command("Open Gemini CLI", F1), ["until", "docs nav"],
        *command("Open Codex", F1), ["until", "og tags"], ["wait", 0.4],
        hook("tool-done", "s-work", tool_input={"command": "ls"}),
        hook("tool-done", "s-done", "Gemini", tool_input={"command": "ls"}),
        hook("stop", "s-done", "Gemini"),
        hook("notify", "s-ask", "Codex", notification_type="permission_prompt",
             message="Codex needs your permission to run cargo publish"),
        *LIST_AGENTS, ["until", "NEEDS YOU"], ["wait", 0.8],
    ]
    # Gemini CLI's row, the last place its tab's name is said.
    scenes = [
        ("m2-agents-urgency", told, {}),
        ("m2-agents-urgency-hover", [*told, ["move", *ON("docs nav", 1, 0, -1)], ["wait", 0.4]], {}),
        ("m2-agents-urgency-light", told, {"background": "eff1f5"}),
        ("m2-agents-urgency-cells", told, {"CELLIX_TIER": "cells"}),
    ]
    # Each scene has sessions of its own, so none finds the last one's.
    for name, steps, extra in scenes:
        cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
        env = {"XDG_CACHE_HOME": cache, "PATH": f"{tools}:{os.environ['PATH']}"}
        background = extra.pop("background", DARK)
        env.update(extra)
        shoot(name, steps, CODE_FILE, size="150x44", env=env, background=background)
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)


def waits(scratch):
    """Cellix's own waits: a language server indexing, in the status bar; a search under way,
    in its view."""
    root = f"{scratch}/indexing"
    os.makedirs(f"{root}/src")
    os.makedirs(f"{root}/.cellix")
    with open(f"{root}/src/main.rs", "w") as f:
        f.write("fn main() {\n    println!(\"hello\");\n}\n")
    # The scripted server, under the name a real one has, told to say it's indexing and stay.
    server = f"{scratch}/rust-analyzer"
    shutil.copy(f"{here}/fake_lsp.py", server)
    os.chmod(server, 0o755)
    with open(f"{root}/.cellix/settings.json", "w") as f:
        json.dump({"language_servers": {"Rust": [server]}}, f)
    env = {"FAKE_LSP_INDEXING": "1"}
    shoot("m3-waits-indexing", [["until", "indexing"], ["wait", 0.4]], "src/main.rs", cwd=root, env=env)
    shoot("m3-waits-indexing-cells", [["until", "indexing"], ["wait", 0.4]], "src/main.rs", cwd=root,
          env={**env, "CELLIX_TIER": "cells"})


def task_scenes(scratch):
    """The Task page on work an agent did: a plan sent back and approved, a turn with failing
    then passing tests, and a later turn, logged as the hooks log them."""
    import time
    root = f"{scratch}/refunds"
    os.makedirs(root)
    project(root)
    start = int(time.time() * 1000) - 40 * 60_000
    log = f"{root}/.cellix/tasks.jsonl"

    def record(**event):
        os.makedirs(os.path.dirname(log), exist_ok=True)
        with open(log, "a") as f:
            f.write(json.dumps(event) + "\n")

    def prompt(at, text):
        event = {"kind": "prompt", "at": at, "session": "s1", "agent": "Claude Code",
                 "text": text, "threads": 0}
        env = {**os.environ, "CELLIX_TASK_EVENT": json.dumps(event), "CELLIX_TASK_ROOT": root}
        subprocess.run([binary, "checkpoint", text[:60]], cwd=root, env=env, capture_output=True)

    plans = f"{scratch}/held/s1"
    os.makedirs(plans)
    with open(f"{plans}/Plan v1.md", "w") as f:
        f.write("# Bound the queue\n\n1. Refuse a push past the limit.\n2. Drop the oldest item when full.\n")
    with open(f"{plans}/Plan.md", "w") as f:
        f.write("# Bound the queue\n\n1. Refuse a push past the limit with `Full`.\n"
                "2. Keep what is queued; the caller decides what to drop.\n3. Test both.\n")
    plan = f"{plans}/Plan.md"
    prompt(start, "Bound the queue so a fast producer can't outrun its consumer.")
    record(kind="plan", at=start + 60_000, session="s1", agent="Claude Code", file=plan)
    record(kind="answer", at=start + 3 * 60_000, file=plan, approved=False,
           note="Don't drop anything. Refuse the push and let the caller decide.")
    record(kind="plan", at=start + 5 * 60_000, session="s1", agent="Claude Code", file=plan)
    record(kind="answer", at=start + 6 * 60_000, file=plan, approved=True, kept=plans)
    with open(f"{root}/src/queue.rs", "a") as f:
        f.write("\nimpl<T> Queue<T> {\n    pub fn len(&self) -> usize {\n        self.items.len()\n    }\n}\n")
    record(kind="test", at=start + 9 * 60_000, session="s1", command="cargo test", passed=False,
           output="test queue::refuses_past_limit ... FAILED\n")
    record(kind="test", at=start + 11 * 60_000, session="s1", command="cargo test", passed=True,
           output="test result: ok. 4 passed\n")
    prompt(start + 20 * 60_000, "Add a test that a full queue still pops in order.")
    with open(f"{root}/src/lib.rs", "w") as f:
        f.write("mod queue;\n")
    show = [["wait", 1.0], *command("show agent task", F1), ["until", "Bound the queue"], ["wait", 0.5]]
    up = ["keys", "\x1b[A"]
    cache = {"env": {"XDG_CACHE_HOME": tempfile.mkdtemp(prefix="cxt", dir="/tmp")}}
    shoot("150-task-page", show, cwd=root, size="150x44", **cache)
    shoot("151-task-plan", [*show, up, up, ["wait", 0.4]], cwd=root, size="150x44", **cache)
    shoot("152-task-turn", [*show, up, up, up, ["wait", 0.6]], cwd=root, size="150x44", **cache)
    shoot("153-task-review-turn", [*show, up, up, up, ["keys", "\r"], ["wait", 1.5]], cwd=root,
          size="150x44", **cache)
    shoot("154-task-narrow", show, cwd=root, size="72x44", **cache)


with tempfile.TemporaryDirectory() as made:
    # The projects are in a stand-in home's dev folder, so a path on screen is ~/dev/queue and
    # never the machine's own temporary folders: the resolved path, as Cellix resolves it.
    HOME = f"{os.path.realpath(made)}/home"
    scratch = f"{HOME}/dev"
    ROOT = f"{scratch}/queue"
    os.makedirs(scratch)
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
    OUTPUT_FROM, OUTPUT_TO, OUTPUT_ASK = (98, 7), (126, 10), (144, 6)
    PROBLEM_COUNT, PROBLEM_ROW = FOOT("⊗", 0), (150, 3)
    # Settings, at the end of the views' row over a sidebar 28 columns wide, before the button
    # that hides the sidebar.
    GEAR, HIDE_SIDEBAR = (22, 0), (25, 0)
    settings = [["click", *GEAR], ["until", "Changes apply"]]

    workspaces(scratch)
    code_intelligence(scratch)
    install_card(scratch)
    transcripts(scratch)
    agent_tabs(scratch)
    agent_names(scratch)
    agent_in_terminal(scratch)
    prompt_keys(scratch)
    agents_by_urgency(scratch)
    waits(scratch)
    task_scenes(scratch)
    team_scenes(scratch)
    reviewer_scenes(scratch)
    ask_scenes(scratch)
    remote_scenes(scratch)
    shoot("01-welcome", [])
    shoot("02-code", [], CODE_FILE)
    # The dark theme on a terminal whose background isn't herdr's: the frame is that
    # background and the sheets keep their step from it.
    shoot("02-code-other-terminal", [], CODE_FILE, background="282c34")

    # More tabs than the strip holds: the wheel over it moves it a tab a notch, the counts at
    # its ends following, and the tab in front stays in front.
    many = ["README.md", "docs/PLAN.md", "Cargo.toml", CODE_FILE]
    shoot("m4-tabs-strip", [["until", "‹"], ["wait", 0.4]], *many, size="84x40")
    shoot("m4-tabs-wheel", [["until", "‹"], ["wheel", 45, 0, -3], ["wait", 0.6]], *many, size="84x40")
    shoot("03-page", [], PAGE_FILE)
    shoot("04-menu-page", [["right", 60, 12]], PAGE_FILE)
    shoot("05-menu-code", [["right", 60, 12]], CODE_FILE)
    shoot("06-menu-explorer", [["right", 12, 2]])
    shoot("07-menu-tab", [["wait", 1.0], ["right", 36, 0], ["wait", 0.3]], CODE_FILE)
    shoot("08-open-file", [["keys", CTRL("p")]], CODE_FILE)
    # A path as a chat or a compiler writes it: the file, and the line it will open at.
    shoot("08-open-file-path", [["keys", CTRL("p")], ["until", "Search files"], ["keys", "src/queue.rs:12"]], PAGE_FILE)
    # Shift+Option+Left in the palette's own field: what's selected is drawn as in the file.
    shoot("08-open-file-select", [["keys", CTRL("p")], ["until", "Search files"], ["keys", "src/queue"],
                                  ["keys", "\\e[1;4D"], ["wait", 0.4]], PAGE_FILE)
    # After another file has been in front, the ones last looked at lead the list.
    again = [["keys", CTRL("p")], ["until", "Search files"], ["keys", "readme"], ["until", "README.md"],
             ["keys", "\r"], ["wait", 0.6], ["keys", CTRL("p")], ["wait", 0.4]]
    shoot("08-open-file-recent", again, CODE_FILE)
    shoot("09-commands", [["keys", F1], ["until", PALETTE_UP], ["keys", "op"]], CODE_FILE)
    # A command run from the palette leads it the next time.
    ran = [["keys", F1], ["until", PALETTE_UP], ["keys", "word wrap"], ["until", "Toggle Word Wrap"],
           ["keys", "\r"], ["wait", 0.5], ["keys", F1], ["wait", 0.4]]
    shoot("09-commands-recent", ran, CODE_FILE)
    # Past the twelfth row, the list follows the pick and a thumb says where it is.
    shoot("09-commands-scrolled", [["keys", F1], ["until", PALETTE_UP], *[["keys", "\x1b[6~"]] * 12, ["wait", 0.4]],
          CODE_FILE)
    shoot("10-branches", [["wait", 1.5], ["click", *FOOT("⎇", 2)], ["wait", 0.5]], CODE_FILE)
    # Once the file is read: a Ctrl+F while it said "Reading…" opened no bar.
    shoot("11-find", [["until", "VecDeque"], ["wait", 0.3], ["keys", CTRL("f")], ["keys", "item"],
                      ["until", "1 of"]], CODE_FILE)
    # Shift+Home in Find's box selects what's typed, as in the file.
    shoot("11-find-select", [["until", "VecDeque"], ["wait", 0.3], ["keys", CTRL("f")], ["keys", "item"],
                             ["until", "1 of"], ["keys", "\\e[1;2H"], ["wait", 0.4]], CODE_FILE)
    # A drag across what's typed in Find's box selects it, as in the file.
    shoot("11-find-drag", [["until", "VecDeque"], ["wait", 0.3], ["keys", CTRL("f")], ["keys", "push_back(item)"],
                           ["until", "1 of"], ["drag", *ON("push_back", 2), *ON("push_back", 12)],
                           ["wait", 0.4]], CODE_FILE)
    shoot("11-find-select-light", [["until", "VecDeque"], ["wait", 0.3], ["keys", CTRL("f")], ["keys", "item"],
                                   ["until", "1 of"], ["keys", "\\e[1;2H"], ["wait", 0.4]], CODE_FILE,
          background="eff1f5")
    shoot("12-search", [["click", *SEARCH]], CODE_FILE)
    replacing = [["keys", CTRL("r")], ["keys", "item"], ["keys", "\t"], ["keys", "entry"]]
    shoot("11-replace", replacing, CODE_FILE)
    shoot("12-search-switches", [["click", *SEARCH], ["keys", "item"], ["keys", "\x1bw"], ["wait", 0.6]], CODE_FILE)
    shoot("13-review", [["keys", "\u0007"], ["wait", 1.2]], CODE_FILE)
    AGENTS_ICON, QUIT = ON("◎", 0), FOOT("Quit")
    shoot("13-strip-hover", [["wait", 0.8], ["move", *ON("◍", 0)], ["wait", 0.4]], CODE_FILE)
    shoot("13-agents-click", [["wait", 1.5], ["click", *AGENTS_ICON], ["wait", 1.5]], CODE_FILE)
    # Quit is at the far end of the bar at the foot, named on the row over it.
    shoot("13-quit-hover", [["wait", 6], ["move", *QUIT], ["wait", 1.2]], CODE_FILE)
    shoot("13-quit-beside-pane", [["wait", 6], ["click", *AGENTS_ICON], ["wait", 3.5], ["move", *QUIT], ["wait", 1.2]], CODE_FILE)
    git_tab = [["click", *REVIEW], ["wait", 0.8]]
    shoot("13-source-hover", [*git_tab, ["move", *ON("PLAN.md")], ["wait", 0.3]], CODE_FILE)
    shoot("13-source-diff", [*git_tab, ["click", *ON("PLAN.md")], ["wait", 1.2]], CODE_FILE)
    shoot("13-source-staged", [*git_tab, ["move", *ON("PLAN.md")], ["wait", 0.4], ["click", *ON("+", 0)],
                               ["wait", 0.8], ["move", *ON("STAGED CHANGES", 2, 1)]], CODE_FILE)
    shoot("13-stacked-pane", [*LIST_AGENTS, ["wait", 0.6]], CODE_FILE, size="72x44")
    shoot("13-source-control", [["click", *REVIEW], ["wait", 0.8]], CODE_FILE)
    shoot("13-source-message", [["click", *REVIEW], ["wait", 0.8], ["click", *ON("Message")],
                                ["keys", "Say what changed and why"], ["move", *ON("Commit")], ["wait", 0.3]], CODE_FILE)
    # Shift+Option+Left twice in the commit message.
    shoot("13-source-message-select", [["click", *REVIEW], ["wait", 0.8], ["click", *ON("Message")],
                                       ["keys", "Say what changed and why"], ["keys", "\\e[1;4D"],
                                       ["keys", "\\e[1;4D"], ["wait", 0.4]], CODE_FILE)
    # The Agents list with an agent in it: one in a tab of its own, as the list has only those.
    # A stand-in for Claude Code that names itself and gives its hook its session from inside
    # its tab as it starts, as the plugin does, and waits.
    agents_bin = f"{scratch}/agents-bin"
    os.makedirs(agents_bin, exist_ok=True)
    said = json.dumps({"session_id": "s-one", "cwd": ROOT})
    with open(f"{agents_bin}/claude", "w") as f:
        f.write("#!/bin/sh\nprintf '\\033]0;pricing page\\007'\n"
                f"printf '%s' '{said}' | '{binary}' hook session-start\nexec sleep 300\n")
    os.chmod(f"{agents_bin}/claude", 0o755)
    shoot("14-agents", [*command("Open Claude Code", F1), ["until", "pricing"], ["wait", 0.4],
                        *LIST_AGENTS, ["until", "IDLE"], ["wait", 0.5]], CODE_FILE,
          env={"PATH": f"{agents_bin}:{os.environ['PATH']}"})
    # The + after the pane's tabs drops a menu of what it starts: Terminal, the agents (one
    # not installed dimmed), the roles, a new worktree. Pointed at, never pressed: a row
    # starts a real agent.
    START_MENU = [*LIST_AGENTS, ["wait", 0.5], ["click", *ON("+", 0)], ["until", "ROLES"]]
    shoot("14-start-menu", [*START_MENU, ["move", *ON("Tester")], ["wait", 0.4]], CODE_FILE)
    shoot("14-start-menu-light", [*START_MENU, ["wait", 0.3]], CODE_FILE, background="eff1f5")
    # New Worktree, with a branch's name typed and never created: it would start an agent.
    # Not `command`: what it waits to be gone, the command's name, is the dialog's title.
    new_worktree = [["keys", F1], ["until", PALETTE_UP], ["keys", "new worktree"], ["until", "New Worktree"],
                    ["keys", "\r"], ["until", "Branch name"], ["keys", "fix/pop-order"],
                    ["until", "fix-pop-order"], ["move", *ON("Create")], ["wait", 0.4]]
    shoot("14-new-worktree", new_worktree, CODE_FILE)
    # A name longer than the box, with the caret moved back to its second word: the box shows
    # the part the caret is in, cut short of its far edge.
    long_name = ("fix/the-pop-order-of-the-bounded-queue-when-a-fast-producer-outruns-a-slow-consumer"
                 "-and-the-buffer-fills-before-anyone-drains-it")
    # The note under the box names the folder from its start, which the screen's text has;
    # the box's own letters there are only as many as its cells.
    shoot("14-new-worktree-long", [*new_worktree[:6], ["keys", long_name], ["until", "worktrees/fix-the-pop"],
                                   ["keys", "\x1b[H"], ["keys", "\x1bf\x1bf"], ["wait", 0.4]], CODE_FILE)
    # A project with a worktree beside it, whose row is right-clicked: Remove Worktree… is
    # offered, and not pressed.
    shop = f"{scratch}/shop"
    os.makedirs(shop)
    with open(f"{shop}/README.md", "w") as f:
        f.write("# Shop\n")
    git(shop, "init", "-q", "-b", "main")
    git(shop, "add", "-A")
    git(shop, "commit", "-qm", "First")
    git(shop, "worktree", "add", "-q", "-b", "fix/pop-order", f"{scratch}/shop.worktrees/fix-pop-order")
    shoot("14-worktree-menu", [*LIST_AGENTS, ["wait", 0.5], ["right", *ON("fix/pop-order")],
                               ["until", "Remove Worktree"], ["move", *ON("Remove Worktree")], ["wait", 0.4]],
          "README.md", cwd=shop)
    shoot("15-settings-appearance", settings, CODE_FILE)
    # Settings' pages, by their names in the list at its left.
    EDITOR_PAGE = ["click", *ON("Editor")]
    shoot("16-settings-editor", [*settings, EDITOR_PAGE], CODE_FILE)
    shoot("17-settings-integrations", [*settings, ["click", *ON("Integrations")]], CODE_FILE)
    shoot("18-settings-keyboard", [*settings, ["click", *ON("Keyboard")]], CODE_FILE)
    shoot("19-dialog", [["keys", "x"], ["keys", CTRL("q")]], CODE_FILE)
    # Workflow: tasks, Open Recent, and Settings' Editor and Keyboard sections, reached through
    # the palette so they don't hang on where the gear is.
    palette = lambda words: [*command(words, F1), ["wait", 0.5]]
    shoot("130-run-task", palette("run task"), CODE_FILE)
    tasks = f"{scratch}/tasks"
    os.makedirs(f"{tasks}/src")
    with open(f"{tasks}/src/main.c", "w") as f:
        f.write("int main(void) {\n    return count;\n}\n")
    with open(f"{tasks}/Makefile", "w") as f:
        f.write("build:\n\t@echo 'src/main.c:2:12: error: use of undeclared identifier count'"
                "; echo 'src/main.c:1:5: warning: unused parameter'; exit 2\n")
    shoot("131-task-failed", [*palette("run task"), ["wait", 1.0], ["keys", "\r"], ["wait", 4.0]], "src/main.c",
          cwd=tasks)
    shoot("132-task-problems", [*palette("run task"), ["keys", "\r"], ["wait", 2.5],
                                ["click", *FOOT("⊗", 0)], ["wait", 0.6]], "src/main.c", cwd=tasks)
    # The pointer on a problem: its row washed, with the button at its end inside the wash
    # and the message cut short of it.
    shoot("132-task-problem-hover", [*palette("run task"), ["keys", "\r"], ["wait", 2.5],
                                     ["click", *FOOT("⊗", 0)], ["wait", 0.6],
                                     ["move", ["undeclared identifier"]], ["wait", 0.4]],
          "src/main.c", cwd=tasks)
    shoot("133-open-recent", palette("open recent"), CODE_FILE)
    to_section = lambda tabs: [*palette("open settings"), ["until", "Changes apply"],
                               *[["keys", "\t"]] * tabs, ["keys", "\r"], ["wait", 0.5]]
    shoot("134-settings-editor", to_section(2), CODE_FILE)
    shoot("135-settings-keyboard", to_section(4), CODE_FILE)
    shoot("136-settings-keys-search", [*to_section(4), ["keys", "task"], ["wait", 0.4]], CODE_FILE)
    # Shift+Left twice in Settings' filter.
    shoot("136-settings-filter-select", [*to_section(4), ["keys", "task"], ["keys", "\\e[1;2D"],
                                         ["keys", "\\e[1;2D"], ["wait", 0.4]], CODE_FILE)
    shoot("137-settings-keys-record", [*to_section(4), ["keys", "rerun"], ["wait", 0.4],
                                       *[["keys", "\t"]] * 6, ["keys", "\r"], ["keys", "\x1b\x12"],
                                       ["wait", 0.4]], CODE_FILE)
    # The tasks folder and its file recent, beside Start. Each scene has a cache of its own,
    # so the list the scenes above left isn't there: this one is written for it.
    fresh = f"{scratch}/fresh"
    os.makedirs(fresh)
    recent_cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
    os.makedirs(f"{recent_cache}/cellix")
    with open(f"{recent_cache}/cellix/recent.json", "w") as f:
        json.dump({"folders": [tasks, ROOT], "files": [f"{tasks}/src/main.c", f"{ROOT}/{CODE_FILE}"]}, f)
    shoot("138-welcome-recent", [], size="150x44", cwd=fresh, env={"XDG_CACHE_HOME": recent_cache})
    shoot("20-prompt", [["right", 12, 2], ["click", *ON("New File")]])
    shoot("21-selection", [["double", *ON("Self {")], ["keys", CTRL("d")]], CODE_FILE)
    shoot("22-narrow", [], CODE_FILE, size="64x30")
    shoot("23-narrow-menu", [["right", 20, 10]], PAGE_FILE, size="64x30")
    shoot("24-light-page", [], PAGE_FILE, background="eff1f5")
    shoot("25-light-menu", [["right", 60, 12]], PAGE_FILE, background="eff1f5")
    shoot("25-light-open-file", [["keys", CTRL("p")], ["until", "Search files"], ["keys", "que"]], PAGE_FILE,
          background="eff1f5")
    shoot("26-light-settings", settings, CODE_FILE, background="eff1f5")
    shoot("27-hover-rail", [["move", *REVIEW]], CODE_FILE)
    # The sidebar's own button, at the end of the views: pointed at, and pressed, after which
    # the same window in outline starts the tabs, pointed at too. The rail it ends without
    # pictures is the cells' alone, which this terminal doesn't draw: the tests read it.
    # Pointed at once the sidebar is there: a move before Cellix asks for the mouse is lost.
    shoot("27-sidebar-hide-hover", [["until", "QUEUE"], ["move", *HIDE_SIDEBAR], ["wait", 0.4]], CODE_FILE,
          expect="QUEUE")
    shoot("27-sidebar-hidden", [["until", "QUEUE"], ["click", *HIDE_SIDEBAR], ["gone", "QUEUE"], ["move", 1, 0],
                                ["wait", 0.4]], CODE_FILE)
    # A window too narrow for the button at the views' end, 84 columns: the sidebar keeps its
    # 21 and the button starts the tabs, filled, where the one that brings it back stands.
    shoot("27-sidebar-narrow-hover", [["until", "QUEUE"], ["move", 22, 0], ["wait", 0.4]], CODE_FILE,
          size="84x40", expect="QUEUE")
    shoot("28-hover-tree", [["move", 12, 3]], CODE_FILE)
    # On the tab's name, by its words: the fixed cell (38, 0) came to be its × once a tab was
    # as wide as what it paints, and the picture was 41's.
    shoot("29-hover-tab", [["move", *ON("queue.rs", 3)]], CODE_FILE)
    shoot("30-hover-menu", [["right", 60, 12], ["move", 64, 13]], CODE_FILE)
    shoot("31-hover-palette", [["keys", CTRL("p")], ["move", 60, 7]], CODE_FILE)
    shoot("32-hover-toolbar", [["move", *ON("▦", 0)]], PAGE_FILE)
    shoot("33-hover-branch", [["move", *FOOT("⎇", 2)]], CODE_FILE)
    # The branch beside the folder's name, in the row under the views.
    # Once git has answered (its dots in the tree), so the chip isn't pictured half loaded;
    # and a moment under the pointer, as a person's hover has.
    shoot("34-hover-header", [["until", "•"], ["move", *ON("QUEUE main", 7)], ["wait", 0.6]],
          CODE_FILE)
    shoot("35-hover-review", [["keys", "\u0007"], ["until", "Accept"], ["wait", 0.6],
                              ["move", *ON("docs/PLAN.md")], ["wait", 0.4]], CODE_FILE)
    shoot("36-hover-accept", [["keys", "\u0007"], ["until", "Accept"], ["wait", 0.6],
                              ["move", *ON("Accept")], ["wait", 0.4]], CODE_FILE)
    shoot("37-hover-settings", [*settings, EDITOR_PAGE, ["move", *ON("After a pause")]], CODE_FILE)
    # On Don't Save by its words: a cell fixed by number fell under the dialog once its air
    # was evened out. The buttons stand lifted half a row over the row their words are
    # counted on, so the middle of that row is under them: the pointer goes half a row up.
    shoot("38-hover-dialog", [["keys", "x"], ["keys", CTRL("q")], ["until", "Don't Save"],
                              ["move", *ON("Don't Save", 2, -0.5)], ["wait", 0.4]], CODE_FILE)
    shoot("39-hover-agents", [*LIST_AGENTS, ["wait", 0.5], ["move", *ON("Claude Code", 2)]], CODE_FILE)
    shoot("41-hover-tab-close", [["move", *ON("×", 0)]], CODE_FILE)
    # A language's Install, under the pointer and never pressed.
    shoot("42-hover-install", [*settings, EDITOR_PAGE, ["move", *ON("Install")]], CODE_FILE)
    # Where Homebrew isn't on the PATH, what can't be installed here says what it needs and
    # has its command to copy, under the pointer and never pressed.
    shoot("42-settings-needs", [*settings, EDITOR_PAGE, ["move", *ON("Copy Command")]], CODE_FILE,
          env={"PATH": "/usr/bin:/bin:/usr/sbin:/sbin"})
    shoot("43-light-hover-menu", [["right", 60, 12], ["move", 64, 13]], CODE_FILE, background="eff1f5")
    # Pointed at once the count is in, which moves the arrows when it comes.
    shoot("40-hover-find", [["keys", CTRL("f")], ["keys", "item"], ["until", "1 of"], ["move", *ON("↑", 0)],
                            ["wait", 0.4]], CODE_FILE)

    FORMULA, TABLE = ON("Σ", 0), ON("▦", 0)
    shoot("44-note", [], NOTE_FILE, size="150x44", background="eff1f5")
    shoot("45-note-dark", [], NOTE_FILE, size="150x44")
    # Each waits for what it opens: a click before the page is drawn lands on nothing.
    shoot("46-formula-open", [["until", "92 words"], ["click", 88, 14], ["wait", 0.5]], NOTE_FILE, size="150x44",
          expect="$$")
    shoot("47-math-panel", [["click", *FORMULA]], NOTE_FILE, size="150x44", background="eff1f5",
          expect="Fraction")
    # A size in the picker under it: twenty columns right of the button and six rows down.
    shoot("48-table-picker", [["click", *TABLE], ["move", *ON("▦", 20, 6)]], NOTE_FILE, size="150x44",
          expect="columns")
    # In a window this wide the page stands eight columns in from where its pane's margin is.
    shoot("49-table-menu", [["until", "92 words"], ["right", 49, 29]], NOTE_FILE, size="150x44",
          background="eff1f5", expect="Delete Row")
    # A debug build presents a page of math slowly, so each step is given time to land.
    comment = [["wait", 1.5], ["click", 90, 5], ["wait", 0.6], *command("add comment", F1)]
    comment += [["until", "Enter sends"], ["keys", "Is this true for multi-head attention?\r"]]
    comment += [["gone", "Enter sends"], ["wait", 0.5]]
    shoot("50-comments", comment, NOTE_FILE, size="150x44", background="eff1f5")
    # A reply being written, a word of it double-clicked: the word selected as the file selects
    # one, the caret at its end.
    reply = [*comment, ["click", *ON("Reply")], ["until", "Enter sends"],
             ["keys", "Only when the heads share a cache"], ["wait", 0.5],
             ["double", *ON("share", 2)], ["wait", 0.8]]
    shoot("50-reply-double-click", reply, NOTE_FILE, size="150x44", expect="Enter sends")
    # With Vim's keys: the comment box opens in Insert, Esc goes to Normal, whose mode shows at
    # the box's right, and b goes back a word. Opened from the selection's Comment chip, as the
    # file's own keys are Vim's (or Neovim's) here.
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/settings.json", "w") as f:
        json.dump({"keymap": "vim"}, f)
    normal = [["wait", 3.0], ["double", *ON("roma sfm", 6)], ["wait", 0.5],
              ["click", *ON("Comment (Alt")], ["until", "Enter sends"],
              ["keys", "Is this true for multi-head attention"], ["keys", "\\e"], ["until", "NORMAL"],
              ["keys", "b"], ["wait", 0.8]]
    shoot("50-comment-vim-normal", normal, NOTE_FILE, size="150x44", cellix=True, expect="NORMAL")
    # Vim's keys are this scene's alone, whichever scenes are taken after it.
    if os.path.exists(f"{ROOT}/.cellix/settings.json"):
        os.unlink(f"{ROOT}/.cellix/settings.json")
    # Being typed: the caret stands after the space just typed.
    typing = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
              ["until", "Enter sends"], ["keys", "Is this true for "], ["wait", 1.5]]
    shoot("50-comment-typing", typing, NOTE_FILE, size="150x44", background="eff1f5")
    # Shift+Option+Left twice, as a terminal sends it (CSI 1;4D): the last two words selected,
    # drawn as the file draws its selection, the caret where the selection was made to.
    select = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
              ["until", "Enter sends"], ["keys", "Is this true for multi-head attention"],
              ["keys", "\\e[1;4D"], ["keys", "\\e[1;4D"], ["wait", 1.0]]
    shoot("50-comment-shift-select", select, NOTE_FILE, size="150x44", expect="Enter sends")
    shoot("50-comment-shift-select-light", select, NOTE_FILE, size="150x44", background="eff1f5",
          expect="Enter sends")
    # A name between colons is its emoji once its second colon is typed, and one half typed
    # is offered under the words, in place of the hint. A name's cells are cut short by the
    # next emoji's, since small text takes less room than its cells, so less of it is waited for.
    # With no comment made yet, so the one being written starts at the pane's top whichever
    # scenes ran before, and the face is where the next scene clicks.
    if os.path.exists(f"{ROOT}/.cellix/threads.json"):
        os.unlink(f"{ROOT}/.cellix/threads.json")
    named = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
             ["until", "Enter sends"], ["keys", "Looks right :+1: and fast :roc"],
             ["until", ":rock"], ["wait", 0.6]]
    shoot("50-comment-emoji", named, NOTE_FILE, size="150x44")
    # Every emoji, from the face after the eight that stand under the words.
    every = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
             ["until", "Enter sends"], ["keys", "Ship it "], ["wait", 0.6], ["click", *EMOJI_BUTTON],
             ["until", "Search emoji"], ["wait", 0.6]]
    shoot("50-emoji-picker", every, NOTE_FILE, size="150x44")
    shoot("50-emoji-picker-light", [*every, ["keys", "fire"], ["wait", 0.6]], NOTE_FILE,
          size="150x44", background="eff1f5")
    # A comment with a picture on it, an emoji and a link, and an answer with a link.
    os.makedirs(f"{ROOT}/.cellix/pasted", exist_ok=True)
    shutil.copy(f"{here}/../../docs/images/comments.png", f"{ROOT}/.cellix/pasted/pasted-1.png")
    pictured = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "The card should read like this one .cellix/pasted/pasted-1.png 👀",
         "replies": [{"author": "Claude", "text": "Agree: it follows https://arxiv.org/abs/1706.03762 too."}],
         "resolved": False, "told": 1, "asked": False, "kind": "", "suggested_by": ""},
    ]
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(pictured, f)
    with_picture = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.5]]
    shoot("50-comment-picture", with_picture, NOTE_FILE, size="150x44", cellix=True)
    shoot("50-comment-picture-light", with_picture, NOTE_FILE, size="150x44", background="eff1f5", cellix=True)
    # A right click on a link in a comment: Open Link and Copy Link, at the pointer.
    linked = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "The proof is at https://example.com/notes/softmax, in its second part.",
         "replies": [{"author": "Claude", "text": "Agree: https://example.com/papers/attention says so too."}],
         "resolved": False, "told": 1, "asked": False, "kind": "", "suggested_by": ""},
    ]
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(linked, f)
    link_menu = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.0],
                 ["right", ["example.com/notes", 6]], ["wait", 0.6]]
    shoot("50-comment-link-menu", link_menu, NOTE_FILE, size="150x44", cellix=True)
    # Comments naming each other: each card's number, muted, last in its header, and a mention
    # of another, "#1" in a comment and "Comment 1" in an agent's answer, a link.
    mentions = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "Say which normaliser this assumes.", "replies": [],
         "resolved": False, "told": 0, "asked": False, "kind": "Clarity", "suggested_by": ""},
        {"id": 2, "file": NOTE_FILE, "line": 3, "quote": "for inputs of $n$ tokens",
         "text": "The same gap as #1, for the cost claim.",
         "replies": [{"author": "Claude", "text": "Agree: one sentence covers both, as Comment 1 asks."}],
         "resolved": False, "told": 1, "asked": False, "kind": "Evidence", "suggested_by": ""},
    ]
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(mentions, f)
    mentioned = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.0]]
    shoot("50-comment-mentions", mentioned, NOTE_FILE, size="150x44", cellix=True)
    # The reviewer's comment with what Validate said of it and Polish's words for it, each in
    # its box under the words, as they're kept with the comment.
    wordy = ("I wonder whether this needs to say which normaliser it assumes, since a reader "
             "could take it for a sparse one.")
    validated = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": wordy, "replies": [], "resolved": False, "told": 0, "asked": False,
         "kind": "Clarity", "suggested_by": "",
         "validation": {"verdict": "holds_up", "kind": "Clarity", "by": "Claude Code", "on": wordy,
                        "why": "The section never names the normaliser, and a sparse one breaks the sum."},
         "polished": {"text": "Say which normaliser this assumes: a sparse one breaks the sum.",
                      "by": "Claude Code", "from": 21, "on": wordy}},
        {"id": 2, "file": NOTE_FILE, "line": 3, "quote": "for inputs of $n$ tokens",
         "text": "Where is the cost claim measured?", "replies": [], "resolved": False,
         "told": 0, "asked": False, "kind": "Evidence", "suggested_by": "",
         "validation": {"verdict": "already_answered", "kind": "Evidence", "by": "Claude Code",
                        "on": "Where is the cost claim measured?",
                        "why": "The section Cost gives the measurement, on 4k tokens."}},
    ]
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(validated, f)
    shoot("50-comment-validated", mentioned, NOTE_FILE, size="150x44", cellix=True,
          expect="Holds up")
    shoot("50-comment-validated-light", mentioned, NOTE_FILE, size="150x44", cellix=True,
          expect="Holds up", background="eff1f5")
    # A comment with the reviewer's reply and an agent's: the pointer on the reviewer's shows
    # its Edit and Delete at the end of the line over it, and on the agent's only Delete. Then
    # the reviewer's being changed, in the box replies are written in.
    replied = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "Say which normaliser this assumes.",
         "replies": [{"author": "You", "text": "Softmax, or a sparse one too?"},
                     {"author": "Claude", "text": "Softmax only; a sentence now says so."}],
         "resolved": False, "told": 1, "asked": False, "kind": "Clarity", "suggested_by": ""},
    ]
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(replied, f)
    shown = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.0]]
    hovered = [*shown, ["move", *ON("or a sparse", 2)], ["wait", 0.6]]
    shoot("50-reply-edit-hover", hovered, NOTE_FILE, size="150x44", cellix=True, expect="or a sparse")
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(replied, f)
    agent = [*shown, ["move", *ON("a sentence now", 2)], ["wait", 0.6]]
    shoot("50-reply-delete-hover", agent, NOTE_FILE, size="150x44", cellix=True, expect="sentence now")
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(replied, f)
    # Edit is painted words with no cells of their own, so it's found from the reply's words:
    # the row over them, at the card's right.
    editing = [*hovered, ["click", *ON("Softmax, or", 41, -1)], ["until", "Enter saves"], ["keys", " Or both?"],
               ["wait", 0.6]]
    shoot("50-reply-editing", editing, NOTE_FILE, size="150x44", cellix=True, expect="Save")
    # A card's words selected with a drag, tinted as a page's selection is, and a right click on
    # them: Copy, Copy Comment and Copy Quote. Each scene has the comments written afresh, so
    # none stands on what the one before left.
    def mentions_again():
        os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
        with open(f"{ROOT}/.cellix/threads.json", "w") as f:
            json.dump(mentions, f)
    selected = [*mentioned, ["drag", *ON("same gap", 0), *ON("cost claim", 10)], ["wait", 1.2]]
    mentions_again()
    shoot("50-comment-selected", selected, NOTE_FILE, size="150x44", cellix=True)
    copy_menu = [*selected, ["right", *ON("cost claim", 2)], ["wait", 0.6]]
    for scene, extra in (("50-comment-copy-menu", {}),
                         ("50-comment-copy-menu-light", {"background": "eff1f5"})):
        mentions_again()
        shoot(scene, copy_menu, NOTE_FILE, size="150x44", cellix=True, **extra)
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
    comments = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.0]]
    shoot("50-review", comments, NOTE_FILE, size="150x44", cellix=True)
    # The resolved one unfolded under its button, after the two still open, with the pointer
    # on the order's button; and the same in the light theme with it off both.
    unfolded = [*comments, ["click", *REVIEW_RESOLVED], ["wait", 0.6]]
    shoot("50-review-resolved", [*unfolded, ["move", *REVIEW_ORDER], ["wait", 0.4]], NOTE_FILE,
          size="150x44", cellix=True)
    shoot("50-review-resolved-light", unfolded, NOTE_FILE, size="150x44", background="eff1f5", cellix=True)
    # The order's list, dropped from under its button.
    shoot("50-review-order", [*comments, ["click", *REVIEW_ORDER], ["wait", 0.6]], NOTE_FILE,
          size="150x44", cellix=True)
    # The Comments tab's own menu: what's done with the review once it's written.
    shoot("50-review-tab-menu", [*comments, ["right", *ON("Comments", 2)], ["wait", 0.6]],
          NOTE_FILE, size="150x44", cellix=True)
    # The review as Markdown, saved into the project and opened as a page; taken away after,
    # so no later scene's tree has it.
    saved = [*comments, *command("Save Review as Markdown", F1), ["until", "Save review as"],
             ["keys", "\r"], ["until", "Saved the review"], ["wait", 1.5]]
    shoot("50-review-saved", saved, NOTE_FILE, size="150x44", cellix=True)
    if os.path.exists(f"{ROOT}/review.md"):
        os.unlink(f"{ROOT}/review.md")
    # The pointer resting on commented words with the pane shut: their comment on a card
    # under them. The page's words overlap in the screen's text, so the pointer goes by "sum"
    # of "the weights sum to one", the first the screen says.
    glance = [["wait", 1.5], ["move", *ON("sum", 1)], ["until", "Open in Comments"], ["wait", 0.6]]
    shoot("50-comment-glance", glance, NOTE_FILE, size="150x44", cellix=True)
    shoot("50-comment-glance-light", glance, NOTE_FILE, size="150x44", background="eff1f5", cellix=True)
    # A press on the card opens the Comments on that comment.
    shoot("50-comment-glance-open", [*glance, ["click", *ON("Open in Comments", 2)],
                                     ["until", "Check My"], ["wait", 1.0]], NOTE_FILE,
          size="150x44", cellix=True)
    shoot("50-plan-versions", comments, "docs/PLAN.md", size="150x44", cellix=True)
    # Narrow beside the editor: a pill that can't share its header's row goes under it.
    shoot("50-plan-versions-narrow", comments, "docs/PLAN.md", size="96x44", cellix=True)
    # A comment being written, its kind picked from the chips under the hint.
    if os.path.exists(f"{ROOT}/.cellix/threads.json"):
        os.unlink(f"{ROOT}/.cellix/threads.json")
    chips = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
             ["until", "Enter sends"], ["keys", "Only with softmax weights"], ["wait", 0.6],
             ["click", *ON("Evidence")], ["wait", 0.6]]
    shoot("50-flow-chips", chips, NOTE_FILE, size="150x44")
    # How a comment goes, on one page: one waiting to reach an agent, saying how; one an agent
    # says it addressed, with Resolve and Not Done; one on words reworded since, which keeps
    # what it was made on; and a suggestion to keep, ask about or dismiss. Each says how long
    # ago it was written, where that's known.
    now = int(time.time())
    flow = [
        {"id": 1, "file": NOTE_FILE, "line": 3, "quote": "the weights sum to one",
         "text": "Is this true for multi-head attention?", "replies": [], "resolved": False,
         "kind": "Clarity", "suggested_by": "", "at": now - 300},
        {"id": 2, "file": NOTE_FILE, "line": 3, "quote": "for inputs of $n$ tokens",
         "text": "Say how large n gets in the experiments.",
         "replies": [{"author": "Claude", "text": "Addressed: the sizes are in the table now.",
                      "at": now - 120}],
         "resolved": False, "told": 0, "kind": "Evidence", "suggested_by": "", "at": now - 7200},
        {"id": 3, "file": NOTE_FILE, "line": 3, "quote": "and the weights add to one, for inputs",
         "text": "Always exactly one?", "replies": [], "resolved": False, "told": 0,
         "kind": "Correctness", "suggested_by": "", "at": now - 90000},
        {"id": 4, "file": NOTE_FILE, "line": 7, "quote": "a softmax over similarity scores",
         "text": "Say which similarity: dot product, scaled.", "replies": [], "resolved": False,
         "told": 0, "kind": "Clarity", "suggested_by": "Claude", "at": now - 60},
    ]
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(flow, f)
    flowing = [["wait", 1.0], *command("show comments", F1), ["until", "Addressed by"], ["wait", 1.0]]
    shoot("50-flow-cards", flowing, NOTE_FILE, size="150x44", cellix=True)
    shoot("50-flow-cards-light", flowing, NOTE_FILE, size="150x44", background="eff1f5", cellix=True)
    # A press on a kind's pill lists that kind alone; its pill is the accent's.
    shoot("50-flow-kind", [*flowing, ["click", *ON("Evidence")], ["gone", "Always exactly"],
                           ["wait", 0.6]], NOTE_FILE, size="150x44", cellix=True)
    # The pill on the reworded one opens what changed since, word by word.
    shoot("50-flow-changes", [*flowing, ["click", *ON("Changed since", 2)], ["wait", 0.8]],
          NOTE_FILE, size="150x44", cellix=True)
    # Next Comment from the palette: the second in the document's order, its words selected
    # and its card lit, and the bar says which of how many.
    shoot("50-flow-next", [*flowing, *command("next comment", F1), ["until", "Comment 1 of"],
                           *command("next comment", F1), ["until", "Comment 2 of"], ["wait", 0.6]],
          NOTE_FILE, size="150x44", cellix=True)
    # Narrow, as Claude Code docks it, where cards are short until one is lit.
    shoot("57-flow-dock", flowing, NOTE_FILE, size="64x40", cellix=True)
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(review, f)
    # Every comment on the file settled: the pane says none is open, says how one is made and
    # offers the settled ones, each on a row of its own. Then with them shown, under those rows.
    settled = [dict(thread, resolved=True) for thread in review
               if thread["file"] == NOTE_FILE and not thread["suggested_by"]]
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(settled, f)
    none_open = [["wait", 1.0], *command("show comments", F1), ["until", "No open comments"],
                 ["wait", 1.0]]
    shoot("50-comments-all-resolved", none_open, NOTE_FILE, size="150x44", cellix=True)
    shoot("50-comments-resolved-shown", [*none_open, ["click", *SHOW_RESOLVED], ["wait", 0.6]],
          NOTE_FILE, size="150x44", cellix=True)
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(review, f)
    shoot("51-agent-list", [*LIST_AGENTS, ["wait", 1.0]], NOTE_FILE, size="150x44")
    shoot("51-inbox-empty", [*SHOW_INBOX, ["wait", 0.6]], NOTE_FILE, size="150x44")
    # More agents: stand-ins for OpenCode, Copilot CLI and Cursor's agent on the PATH put them
    # under START. Copilot's, started, runs the hooks `cellix setup copilot` wrote into the
    # tour's home, as Copilot CLI runs them, and asks to run something.
    standins = tempfile.mkdtemp(prefix="cx-agents")
    for program, script in [("opencode", PLAIN_STANDIN), ("cursor-agent", PLAIN_STANDIN),
                            ("copilot", COPILOT_STANDIN), ("kiro-cli", KIRO_STANDIN)]:
        with open(f"{standins}/{program}", "w") as f:
            f.write(script)
        os.chmod(f"{standins}/{program}", 0o755)
    subprocess.run([binary, "setup", "copilot"], env={**os.environ, "HOME": HOME},
                   capture_output=True, check=True)
    more = {"PATH": f"{standins}:{os.environ['PATH']}"}
    shoot("51-agent-list-more", [*LIST_AGENTS, ["wait", 1.0]], NOTE_FILE, size="150x44", env=more,
          expect="Kiro CLI")
    asks = [*LIST_AGENTS, ["click", *ON("Copilot CLI", 1, 0, -1)], ["until", "Allow cargo publish? (y/n)"],
            *LIST_AGENTS, ["wait", 1.0]]
    shoot("51-agent-copilot-asks", asks, NOTE_FILE, size="150x44", env=more, expect="needs you")
    # Kiro CLI, installed: the + menu lists it with its key, and Settings › Integrations has
    # its card, its hooks not yet installed. Pointed at, never pressed.
    shoot("51-start-menu-more", [*START_MENU, ["move", *ON("Kiro CLI")], ["wait", 0.4]], NOTE_FILE,
          size="150x44", env=more, expect="Kiro CLI")
    shoot("51-settings-kiro", [*settings, ["click", *ON("Integrations")], ["move", *ON("Kiro CLI")],
                               ["wait", 0.4]], NOTE_FILE, size="150x44", env=more,
          expect="Hooks not installed")
    # Its stand-in, started from START, runs the hooks `cellix setup kiro` wrote, as Kiro CLI
    # runs them: its tab is named by the prompt, and it asks to run something. In front, then
    # in the list.
    subprocess.run([binary, "setup", "kiro"], env={**os.environ, "HOME": HOME, **more},
                   capture_output=True, check=True)
    kiro = [*LIST_AGENTS, ["click", *ON("Kiro CLI", 1, 0, -1)], ["until", "Allow cargo publish? [y/n/t]"],
            ["wait", 1.0]]
    shoot("51-agent-kiro", kiro, NOTE_FILE, size="150x44", env=more, expect="Refund route")
    shoot("51-agent-kiro-asks", [*kiro, *LIST_AGENTS, ["wait", 1.0]], NOTE_FILE, size="150x44",
          env=more, expect="needs you")
    shoot("51-agent-kiro-light", kiro, NOTE_FILE, size="150x44", env=more, expect="Refund route",
          background="eff1f5")
    shoot("51-agent-kiro-asks-light", [*kiro, *LIST_AGENTS, ["wait", 1.0]], NOTE_FILE,
          size="150x44", env=more, expect="needs you", background="eff1f5")
    shutil.rmtree(standins, ignore_errors=True)
    shell = [*LIST_AGENTS, ["wait", 0.6]]
    shoot("52-paper", [], PAPER_FILE, size="150x44", background="eff1f5")
    shoot("53-paper-dark", [], PAPER_FILE, size="150x44")
    # Build, the toolbar's last button.
    BUILD = ["click", *ON("▶", 0)]
    build = [BUILD, ["wait", 45]]
    shoot("54-paper-pdf", build, PAPER_FILE, size="170x44", background="eff1f5")
    shoot("55-paper-cite", [["click", *ON("[", 0)]], PAPER_FILE, size="150x44", background="eff1f5")
    shoot("55-paper-menu", [["right", 70, 14]], PAPER_FILE, size="150x44", background="eff1f5")
    # The same on a paper: from the abstract, over a section, a formula and a list's item.
    # The abstract's words are the row under its heading, which comes through as "bsrct".
    paper_across = [["click", ["bsrct", 0, 1]], ["shift", ["94.8", 4]], ["wait", 0.5]]
    shoot("s1-paper-shift-select", paper_across, PAPER_FILE, size="150x44", background="eff1f5",
          expect="Ask (Alt")
    shoot("56-narrow-note", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="100x34")
    # About the size Claude Code docks Cellix at: at most 45% of a window, beside the chat.
    shoot("57-dock-page", [["wait", 1.5]], NOTE_FILE, size="64x40")
    # With the two comments that name each other, so the docked cards show, each with its
    # "· #n": on the project as it's left, the pane said only "No comments yet".
    mentions_again()
    shoot("57-dock-comments", [["wait", 1.5], *command("show comments", F1), ["wait", 1.0]], NOTE_FILE,
          size="64x40", cellix=True, expect="Say which")
    shoot("57-dock-agents", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="64x40")
    shoot("57-dock-menu", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4]], NOTE_FILE, size="64x40")
    # Format by its name: a menu's rows don't stand on the window's, so a place by row and
    # column landed on the rule over it once menus had their padding, and nothing opened.
    shoot("57-dock-menu-format", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4], ["move", *ON("Format", 2)],
                                  ["wait", 0.4]], NOTE_FILE, size="64x40", expect="Bold")
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
    ZOOM_IN = ON("Fit", -3)
    # A debug build reads the picture slowly, and each zoom reads it again.
    zoomed = [["wait", 2.0]] + [["click", *ZOOM_IN], ["wait", 1.0]] * 4 + [["wait", 1.0]]
    shoot("91-preview-zoomed", zoomed, RESULTS, LOSS, size="150x44")
    # The pane's maximize button, at the end of its own tabs, and the window it takes.
    MAXIMIZE = ON("⤢", 0)
    shoot("93-pane-maximized", [["wait", 1.0], ["click", *MAXIMIZE], ["wait", 0.6], ["move", 80, 20]],
          RESULTS, LOSS, size="150x44")
    shoot("94-pane-restore-hover", [["wait", 1.0], ["click", *MAXIMIZE], ["wait", 0.6], ["move", 80, 20],
                                    ["wait", 0.3], ["move", *ON("⤡", 0)]], RESULTS, LOSS, size="150x44")
    shoot("95-editor-maximized", [["wait", 4.0], ["double", 36, 0], ["wait", 0.6]], CODE_FILE, LOSS,
          size="150x44")
    # A tab dragged: held over the right half of the file, and the pane dragged under it.
    TAB = (36, 0)
    shoot("96-tab-drag-split", [["wait", 1.0], ["press", *TAB], ["hold", 70, 12], ["hold", 112, 20]],
          CODE_FILE, RESULTS, size="150x44")
    # results.md's tab by its name, held over queue.rs's: the fixed cell (50, 0) came to be
    # results.md's ×, and the press closed it.
    shoot("97-tab-drag-reorder", [["wait", 1.0], ["until", "results"], ["press", *ON("results", 2)],
                                  ["hold", *ON("queue.rs", 6)], ["hold", *ON("queue.rs", 1)]],
          CODE_FILE, RESULTS, size="150x44")
    # A tab is as wide as what it paints, so its cells hold the start of its name.
    PREVIEW_TAB = ON("Previ")
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
    start_shell = [*command("New Terminal"), ["wait", 1.5]]
    shoot("58-welcome-with-shell", start_shell, **light)
    shoot("59-sample-top", [], WRITING, **light)

    def line_of(path, start, after=0):
        """The line of a file that starts with `start`, counting from one, and `after` more."""
        with open(path, encoding="utf8") as f:
            found = [n for n, line in enumerate(f.read().split("\n"), 1) if line.startswith(start)]
        return found[0] + after

    def to_line(line):
        """Go to Line to the end of the file and back up to `line`, which then stands at the
        top of the page. The wheel landed short of a section, by how fast its notches came."""
        go = lambda n: [["keys", CTRL("l")], ["until", "Go to line"], ["keys", f"{n}\r"],
                        ["gone", "Go to line"]]
        # Once the file is drawn: before, a loaded machine shot its blank ground.
        return [["until", " lines", 60], ["wait", 1.0], *go(99999), *go(line), ["wait", 1.0]]

    sections = [
        ("lists", "### Lists", 0),
        ("callouts", "## Callouts", 0),
        ("table", "## Tables", 0),
        ("math", "## Math", 0),
        ("pictures", "## Pictures", 0),
        ("flow", "## Diagrams", 0),
        ("class", "classDiagram", -1),
        ("charts", "gantt", -1),
        ("tikz", "TikZ, built", 0),
        ("code", "## Code", 0),
        ("footnotes", "## Footnotes", 0),
        ("changes", "## Track changes", 0),
    ]
    for number, (name, start, after) in enumerate(sections):
        steps = to_line(line_of(WRITING, start, after))
        # TikZ is built in the background by a LaTeX engine, which takes a few seconds.
        if name == "tikz":
            steps.append(["wait", 6.0])
        shoot(f"{60 + number}-sample-{name}", steps, WRITING, **light)
    shoot("72-sample-dark", to_line(line_of(WRITING, "## Diagrams")), WRITING, size="150x44")
    # Track Changes at work: Accept and Reject by the suggestion the caret is in, and by the
    # one under the pointer, and the row over the comments that counts them and settles them.
    CHANGES = to_line(line_of(WRITING, "## Track changes"))
    at_change = [*CHANGES, *command("Next Change"), ["until", "Suggestion 1 of 3"], ["wait", 0.5]]
    shoot("71-sample-changes-chips", at_change, WRITING, **light)
    shoot("71-sample-changes-chips-dark", at_change, WRITING, size="150x44")
    # The pointer on the change itself, a row under the chips, which start where it does, and
    # the caret then moved up off it: the chips stay for the pointer.
    pointed = [*at_change, ["move", *ON("Rej", 2, 1, -1)], ["wait", 0.4], ["keys", "\\e[A"],
               ["wait", 0.8]]
    shoot("71-sample-changes-pointed", pointed, WRITING, **light)
    shoot("71-sample-changes-chip-hover", [*at_change, ["move", *ON("Acc", 2, 0, -1)], ["wait", 0.5]],
          WRITING, **light)
    # The pane first, since a page laid out again narrower keeps its top row, not its words.
    row = [["wait", 1.0], *command("show comments", F1), ["until", "3 suggestion"], *CHANGES]
    shoot("71-sample-changes-row", row, WRITING, **light)
    shoot("71-sample-changes-row-dark", row, WRITING, size="150x44")
    shoot("71-sample-changes-row-next", [*row, ["click", *ON("Nex")], ["until", "Suggestion 1 of 3"],
                                         ["wait", 0.5]], WRITING, **light)
    # Narrow, the three buttons take a row of their own under the count.
    shoot("71-sample-changes-row-narrow", row, WRITING, size="84x44", background="eff1f5")
    # An agent's rewrite of commented words, to take in one step or as a suggestion: the
    # only comment its scenes have, as each scene starts with no other scene's.
    rewrite = {"id": 1, "file": "samples/writing.md", "line": line_of(WRITING, "The model"),
               "quote": "improves on the baseline", "text": "By how much? Vague as it stands.",
               "replies": [{"author": "Claude", "text": "Rewrite: improves on the baseline by 3.6 points"}],
               "resolved": False, "told": 1, "asked": False, "kind": "Clarity", "suggested_by": ""}
    os.makedirs(f"{ROOT}/.cellix", exist_ok=True)
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump([rewrite], f)
    rewrite_card = [["wait", 1.0], *command("show comments", F1), ["until", "Accept Re"], *CHANGES]
    shoot("71-sample-rewrite", rewrite_card, WRITING, cellix=True, **light)
    shoot("71-sample-rewrite-dark", rewrite_card, WRITING, size="150x44", cellix=True)
    shoot("71-sample-rewrite-narrow", rewrite_card, WRITING, size="100x44", background="eff1f5",
          cellix=True)
    # A double click on "page" in the title, placed from the "Text" heading under it: the
    # title's measured letters don't come through whole in the screen's text.
    shoot("73-selection-bar", [["double", ["Text", 22, -9, -1]], ["wait", 0.5]], WRITING, **light,
          expect="Ask (Alt")
    # A click, and a Shift+click further down: everything between is selected, across the
    # paragraphs, the heading and the lists, with the buttons that comment on it. Then the same
    # after the wheel has taken the click out of view, in the dark. Measured type drops letters
    # from the screen's text, so the places are rows under the "Text" heading and the "Second"
    # item, whose words come through whole, and the paragraph under "Tables".
    plain_words = ["click", ["Text", 0, 3, -1]]
    across = [plain_words, ["shift", ["Second", 15, 1]], ["wait", 0.5]]
    shoot("s1-page-shift-select", across, WRITING, **light, expect="Ask (Alt")
    scrolled = [plain_words, ["wheel", 90, 20, 14], ["wait", 1.0], ["shift", ["Tables", 12, 3]],
                ["wait", 0.5]]
    shoot("s1-page-shift-select-scrolled", scrolled, WRITING, size="150x44", expect="Ask (Alt")
    # A drag held on the bar at the foot, under the page, which scrolls on its own and takes
    # the selection with it until it's let go: once the page has brought "Tables" up. A held
    # time alone let go before the page had moved when frames were slow under load.
    foot = FOOT("UTF-8")
    held = [["press", ["Text", 0, 3, -1]], ["hold", *foot], ["until", "Tables"], ["release", *foot],
            ["wait", 0.5]]
    shoot("s1-page-drag-scroll", held, WRITING, **light, expect="Ask (Alt")
    shoot("74-outline", [["click", *OUTLINE], ["wait", 0.8]], WRITING, **light)
    shoot("75-data", [], os.path.join(samples, "data.csv"), **light)
    shoot("76-notebook", [], os.path.join(samples, "notebook.ipynb"), **light)
    shoot("77-paper-top", [["wait", 1.0]], PAPER, **light)
    paper_sections = [
        ("math", "\\subsection{Equations}"),
        ("theorems", "\\subsection{Theorems}"),
        ("drawn", "\\subsection{Algorithms}"),
        ("tables", None),
        ("changes", "\\section{Track changes}"),
    ]
    for number, (name, start) in enumerate(paper_sections):
        # The paper's table is in the file it takes in, which opens at its top.
        if start is None:
            shoot(f"{78 + number}-paper-{name}", [["wait", 1.0]],
                  os.path.join(samples, "paper", "results.tex"), **light)
            continue
        shoot(f"{78 + number}-paper-{name}", to_line(line_of(PAPER, start)), PAPER, **light)
    # With no LaTeX on the PATH the drawings say so under their source, 16 px under its box,
    # where they said "Drawing with LaTeX…" for ever (0.11.0's release check, P4). The note
    # is small proportional words, whose letters the screen's text loses, so a pause.
    shoot("80-paper-drawn-no-latex", [*to_line(line_of(PAPER, "\\subsection{Algorithms}")),
                                      ["wait", 3.0]], PAPER,
          env={"PATH": "/usr/bin:/bin"}, **light)
    PAPER_CHANGES = to_line(line_of(PAPER, "\\section{Track changes}"))
    shoot("82-paper-changes-chips", [*PAPER_CHANGES, *command("Next Change"),
                                     ["until", "Suggestion 1 of 3"], ["wait", 0.5]], PAPER, **light)
    shoot("82-paper-changes-row", [["wait", 1.0], *command("show comments", F1),
                                   ["until", "3 suggestion"], *PAPER_CHANGES], PAPER, **light)

    # A window wider than a page is set: the page keeps to its reading width, in the middle,
    # and what's wider than that moves sideways inside it.
    WIDE_FILE = "docs/wide.md"
    with open(f"{ROOT}/{WIDE_FILE}", "w") as f:
        f.write(WIDE)
    wide = {"size": "200x50"}
    # Once its spelling has come back, which draws its dotted marks a moment after the page.
    spelled = [["wait", 1.0], ["settled", "spelling", 60]]
    shoot("160-reading-width", spelled, WIDE_FILE, **wide)
    shoot("161-reading-width-across", [["wait", 1.0], ["tip", 100, 21, 6], ["tip", 100, 30, 4],
                                       ["wait", 0.5]], WIDE_FILE, **wide)
    shoot("162-reading-width-note", spelled, NOTE_FILE, background="eff1f5", **wide)
    shoot("163-reading-width-paper", [["wait", 1.0]], PAPER, **wide)
    shoot("164-reading-width-sample", to_line(line_of(WRITING, "## Tables")), WRITING, **wide)
    # A narrow side pane with more tabs than it has room for: the one in front whole, the
    # others cut short (Comments to its mark, the first ◆ on the top row, so "+" keeps its
    # room), and one of those under the pointer.
    three_tabs = [*LIST_AGENTS, *command("show comments", F1), ["until", "No comments"],
                  *command("New Terminal"), ["wait", 1.5]]
    comments_mark = ON("◆", 0)
    shoot("165-pane-tabs-cut", three_tabs, CODE_FILE)
    shoot("166-pane-tabs-cut-hover", [*three_tabs, ["move", *comments_mark], ["wait", 1.2]],
          CODE_FILE)
    shoot("167-pane-tabs-cut-light", [*three_tabs, ["click", *comments_mark], ["wait", 0.6]],
          CODE_FILE, background="eff1f5")

    # The code view: fold chevrons and chips, sticky scroll, word wrap, bracket pairs. The
    # file opens with the gutter's chevrons in columns 32 and 33, its line n on row n + 1.
    VIEW_FILE = "src/scheduler.rs"
    with open(f"{ROOT}/{VIEW_FILE}", "w") as f:
        f.write(SCHEDULER)
    FOLD_KEYS = "\x1b[91;6u"
    at_line = lambda n: n + 1
    shoot("110-view-fold-hover", [["move", 32, at_line(16)], ["wait", 0.4]], VIEW_FILE)
    # run_due folded first, then new above it, which leaves run_due's row where it was.
    folding = [["click", 60, at_line(16)], ["keys", FOLD_KEYS], ["click", 60, at_line(12)],
               ["keys", FOLD_KEYS], ["wait", 0.3]]
    shoot("111-view-folded", folding, VIEW_FILE)
    shoot("111-view-folded-hover", [*folding, ["move", 32, at_line(11)], ["wait", 0.4]], VIEW_FILE)
    short = {"size": "120x24"}
    # Line 27 at the top, by Go to Line: sixteen notches of the wheel landed on another line
    # each run. There `match`'s last arm is first under `impl`, `run_due` and `while`, and
    # `match` (23) is pinned with them.
    shoot("112-view-sticky", [*to_line(27), ["wait", 0.8]], VIEW_FILE, **short)
    # Each scene opens the file where the last one left it, so these go to the top first.
    home = ["keys", "\x1b[1;5H"]
    shoot("113-view-wrap", [home, ["click", 60, at_line(5)], ["keys", "\x1bz"], ["wait", 0.4]], VIEW_FILE)
    shoot("114-view-brackets", [home, ["click", 54, at_line(16)], ["wait", 0.3]], VIEW_FILE)
    shoot("115-view-light", [*to_line(27), ["wait", 0.8], ["move", 32, 14], ["wait", 0.4]],
          VIEW_FILE, background="eff1f5", **short)
    # The cell tier isn't shot here: term.py keeps only Kitty graphics, so it would draw
    # nothing. The view tests in app/view_tests.rs check the cells.
    # Last, so the broken paper they leave open is in no other scene.
    # A failing run in a terminal, its output selected: the buttons that comment on it or ask
    # the agent about it, as by a selection in a file.
    # Each scene has a cache of its own, or the second rejoins the first one's shell.
    in_shell = lambda cache: {"XDG_CACHE_HOME": cache, "SHELL": "/bin/sh", "PS1": "$ ", "ENV": ""}
    caches = [tempfile.mkdtemp(prefix="cx", dir="/tmp") for _ in range(3)]
    failing = ("printf 'test refuses_once_full ... FAILED\\n\\nthread panicked at src/queue.rs:16:9:\\n"
               "assertion failed: queue.push(4).is_err()\\n  left: Ok(())\\n right: Err(Full)\\n'\r")
    selected = [["keys", CTRL("j")], ["wait", 1.5], ["keys", failing], ["wait", 1.0],
                ["drag", *OUTPUT_FROM, *OUTPUT_TO], ["wait", 0.5]]
    shoot("108-terminal-ask", selected, CODE_FILE, size="150x44", env=in_shell(caches[0]))
    shoot("108-terminal-ask-hover", [*selected, ["move", *OUTPUT_ASK], ["wait", 0.4]], CODE_FILE,
          size="150x44", env=in_shell(caches[1]))
    # Find, with the keyboard in the terminal: what it printed is searched and marked.
    found = [["keys", CTRL("j")], ["wait", 1.5], ["keys", failing], ["wait", 1.0], ["keys", CTRL("f")],
             ["wait", 0.4], ["keys", "queue"], ["wait", 0.5]]
    shoot("108-terminal-find", found, CODE_FILE, size="150x44", env=in_shell(caches[2]))
    # A script in a terminal says it's waiting for the person through OSC 7501, as any program
    # can: the Agents list and the inbox say so as they would an agent's.
    caches += [tempfile.mkdtemp(prefix="cx", dir="/tmp") for _ in range(2)]
    asks = ("printf 'deploying\\n\\033]7501;state=blocked:app=deploy:kind=question:"
            "msg=RGVwbG95IHRvIHByb2Q/\\033\\\\'\r")
    program = [["keys", CTRL("j")], ["wait", 1.5], ["keys", asks], ["wait", 1.0]]
    shoot("108-program-asks-agents", [*program, *LIST_AGENTS, ["wait", 0.5]], CODE_FILE,
          size="150x44", env=in_shell(caches[3]), expect="Deploy to prod?")
    shoot("108-program-asks-inbox", [*program, *SHOW_INBOX, ["wait", 0.6]], CODE_FILE,
          size="150x44", env=in_shell(caches[4]), expect="Deploy to prod?")
    # A command that runs on in a terminal: its tab is named after it, with a shell's mark, and
    # turns. The host looks every half second, and the tab waits a second before it says so.
    caches += [tempfile.mkdtemp(prefix="cx", dir="/tmp")]
    running = [["keys", CTRL("j")], ["wait", 1.5], ["keys", "sleep 60\r"], ["wait", 2.5]]
    shoot("108-terminal-running", running, CODE_FILE, size="150x44", env=in_shell(caches[5]),
          expect="sleep 60")
    # A right click on a web address a terminal printed: Open Link and Copy Link.
    caches += [tempfile.mkdtemp(prefix="cx", dir="/tmp")]
    # Worked out by the shell, so only what it prints says the whole address.
    printed = "printf 'Docs: https://example.com/queue/v%s\\n' $((1+1))\r"
    link = [["keys", CTRL("j")], ["wait", 1.5], ["keys", printed], ["until", "queue/v2"],
            ["wait", 0.5], ["right", ["example.com/queue/v2", 4]], ["wait", 0.6]]
    shoot("108-terminal-link-menu", link, CODE_FILE, size="150x44", env=in_shell(caches[6]),
          expect="Copy Link")
    for cache in caches:
        subprocess.run(["pkill", "-9", "-f", cache])
        shutil.rmtree(cache, ignore_errors=True)
    # A paper that doesn't build: its problem in the list, the pointer on it.
    with open(f"{ROOT}/docs/broken.tex", "w") as f:
        f.write("\\documentclass{article}\n\\begin{document}\nHello \\wrongmacro{} world.\n\\end{document}\n")
    # A loaded machine can take over a minute to build, so this one gets longer.
    problems = [["wait", 2.0], BUILD, ["wait", 100], ["click", *PROBLEM_COUNT], ["wait", 0.6],
                ["move", *PROBLEM_ROW], ["wait", 0.4]]
    shoot("109-problem-ask", problems, "docs/broken.tex", size="170x44", timeout=130)
    git_scenes(scratch)
    # Agent Context: a project with instructions that disagree and an import, a user's own
    # instructions and skill, and a session's transcript, all in the throwaway home.
    CONTEXT = (14, 0)  # the fifth view's icon
    ctx = os.path.realpath(f"{scratch}/service")
    os.makedirs(f"{ctx}/src")
    claude_home = f"{HOME}/.claude"
    ctx_files = {
        f"{ctx}/CLAUDE.md": "@AGENTS.md\n@docs/STYLE.md\n",
        f"{ctx}/AGENTS.md": "# Working here\n\n## Rules\n\n**Never run `cargo build`, `test` directly.** Use `harness/x`.\n",
        f"{ctx}/src/routes.rs": "pub fn routes() {}\n",
        f"{ctx}/.claude/agents/reviewer.md": "---\nname: reviewer\ndescription: Reviews a diff before it lands\n---\n",
        f"{ctx}/.claude/settings.json": '{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "bash harness/guard.sh"}]}]}}',
        f"{claude_home}/CLAUDE.md": "# How I like to work\n\n- Run tests with `cargo test` before saying it's done.\n",
        f"{claude_home}/skills/browser/SKILL.md": "---\nname: terminal-browser\ndescription: A real browser in the terminal\n---\n",
    }
    for path, text in ctx_files.items():
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
    git(ctx, "init", "-q", "-b", "main")
    sessions = f"{claude_home}/projects/" + "".join(c if c.isalnum() else "-" for c in ctx)
    os.makedirs(sessions, exist_ok=True)
    transcript = [
        {"type": "attachment", "attachment": {"type": "instructions", "files": [
            {"path": f"{claude_home}/CLAUDE.md", "type": "User", "content": ctx_files[f"{claude_home}/CLAUDE.md"]},
            {"path": f"{ctx}/CLAUDE.md", "type": "Project", "content": ctx_files[f"{ctx}/CLAUDE.md"]},
            {"path": f"{ctx}/AGENTS.md", "type": "Project", "content": ctx_files[f"{ctx}/AGENTS.md"]}]}},
        {"type": "attachment", "attachment": {"type": "skill_listing",
            "content": "- terminal-browser: A real browser in the terminal\n- cellix:cellix-review: Read the user's comments"}},
        {"type": "api-request-shape", "shape": {"system": [{"type": "text", "text": "You are Claude Code. " * 1800}],
            "tools": [{"name": "Read", "description": "Reads a file. " * 3800}]}},
        {"type": "user", "message": {"content": "Add a refund route"}},
        {"type": "assistant", "message": {"model": "claude-opus-5-5", "content": [
            {"type": "tool_use", "id": "t1", "name": "Read", "input": {"file_path": f"{ctx}/src/routes.rs"}},
            {"type": "tool_use", "id": "t2", "name": "Read", "input": {"file_path": f"{ctx}/AGENTS.md"}}],
            "usage": {"input_tokens": 12, "cache_read_input_tokens": 118000}}},
        {"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "x" * 56000},
            {"type": "tool_result", "tool_use_id": "t2", "content": "y" * 4000}]}},
    ]
    with open(f"{sessions}/s1.jsonl", "w") as f:
        f.write("".join(json.dumps(line) + "\n" for line in transcript))
    show_context = [["click", *CONTEXT], ["wait", 1.2]]
    shoot("81-context", show_context, cwd=ctx, size="150x44")
    shoot("81-context-hover", [*show_context, ["move", *ON("1 CLAUDE.md", 3)], ["wait", 0.3]], cwd=ctx, size="150x44")
    # Checks wrap to however many rows their words take, so they're folded out of the way.
    fold_checks = [["click", *ON("CHECKS")], ["wait", 0.3]]
    shoot("81-context-open", [*show_context, *fold_checks, ["click", *ON("↳ AGENTS.md", 3)], ["wait", 1.0]], cwd=ctx, size="150x44")
    # The agent's name at the head of the view, which lists the others.
    shoot("81-context-codex", [*show_context, ["click", *ON("Claude Cod")], ["wait", 1.2]], cwd=ctx, size="150x44")

if FAILED:
    print(f"{len(FAILED)} of {len(SHOT) + len(FAILED)} scenes failed: {', '.join(FAILED)}", flush=True)
    sys.exit(1)
