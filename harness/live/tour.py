#!/usr/bin/env python3
"""Walks through every surface of the real binary and writes a PNG of each, for a person or
a design reviewer to look at. No window opens, and everything runs against a throwaway
project and home folder, so a tour can't touch real settings.

    tour.py <binary> <out folder>

Beside each PNG goes the screen's text, as `<name>.txt`. A scene can say what its surface
shows (`expect=`), and steps can wait for words rather than a clock (`["until", "words"]`): a
scene that never gets there writes no picture, is listed as failed, and fails the tour. A
picture of the wrong screen has passed for the right one before. `manifest.json` says, for
each scene, which commit its picture is of, when it was taken, and whether it failed, so a
folder of pictures from several runs still says how old each one is.

TOUR_ONLY=a,b takes just those scenes. TOUR_PART=2/3 takes the second third of them all, in
order: the whole tour is longer than one guarded job may run. TOUR_LIST=1 names the scenes
it would take, and takes none.
"""
import json, os, shutil, subprocess, sys, tempfile, time

binary, out = sys.argv[1], os.path.abspath(sys.argv[2])
here = os.path.dirname(os.path.abspath(__file__))
os.makedirs(out, exist_ok=True)
CTRL = lambda c: chr(ord(c) - 96)
RAIL_X = 1
# One row each since the rail's redesign, Files on the first.
SEARCH, OUTLINE, REVIEW = 2, 3, 5  # a row and a half apart, at their middles
# The command palette, opened with Ctrl+Shift+P or F1 and waited for, then a command typed
# and run, and the palette gone with what was typed into it. Each step waits for the screen to
# say the one before it landed: keys sent before the palette is up are typed into the file.
PALETTE_UP = "Type to search"
# The face under a comment being written in a 150-column window, after the eight emoji.
EMOJI_BUTTON = (124, 4)
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
    "09-commands": "Open Settings",
    "10-branches": "Switch branch",
    "100-workspaces": "Resume",
    "101-workspace-hover": "Resume",
    "102-stopped-card": "building the index",
    "103-running-here": "workspaces are open",
    "104-logout-card": "when you disconnect",
    "105-new-workspace-hover": "Resume",
    "107-folder-path": "infra",
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
    "13-source-diff": "SOURCE CONTROL",
    "13-source-hover": "SOURCE CONTROL",
    "13-source-message": "what changed and",
    "13-source-staged": "STAGED CHANGES",
    "13-stacked-pane": "WORKTREES",
    "130-run-task": "Select the task",
    "131-task-failed": "make build",
    "132-task-problems": "undeclared identifier",
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
    "26-light-settings": "Changes apply",
    "30-hover-menu": "Rename Symbol",
    "31-hover-palette": "Search files",
    "35-hover-review": "Accept",
    "36-hover-accept": "Accept",
    "37-hover-settings": "files save",
    "38-hover-dialog": "Save changes",
    "39-hover-agents": "WORKTREES",
    "42-hover-install": "files save",
    "43-light-hover-menu": "Rename Symbol",
    "50-comments": "yours",
    "50-comment-emoji": ":rock",
    "50-emoji-picker": "Search emoji",
    "50-emoji-picker-light": "fire",
    "50-comment-picture": "yours",
    "50-comment-picture-light": "yours",
    "50-plan-versions": "yours",
    "50-plan-versions-narrow": "yours",
    "50-review": "suggested",
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
    "83-git-timeline": "Bound the queue",
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
# About how many scenes there are, to cut the tour into parts of a size.
SCENES_IN_ALL = 195
# Scenes come to, shot, and those that never reached their surface.
SEEN, SHOT, FAILED = [0], [], []
# Which commit the binary is, as far as git can say: the pictures are of that.
COMMIT = subprocess.run(["git", "-C", here, "describe", "--always", "--dirty"], capture_output=True,
                        text=True).stdout.strip()
MANIFEST = f"{out}/manifest.json"
try:
    with open(MANIFEST) as f:
        SCENES = json.load(f)["scenes"]
except (OSError, ValueError, KeyError):
    SCENES = {}


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


def shoot(name, steps, *args, size="120x40", background="1e1e2e", cwd=None, env=None, timeout=70,
          expect=None):
    """Runs one scene and writes `<name>.png` and `<name>.txt`, with `steps` run first. With
    `expect`, only once the screen says it."""
    # TOUR_ONLY=50-review,02-code retakes just those, for a quick look after a change.
    only = os.environ.get("TOUR_ONLY")
    if only and name not in only.split(","):
        return
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
    script = [*steps, *waited, ["wait", 0.3], ["shot", f"{out}/{name}.png"]]
    # A picture from an earlier tour mustn't stand in for one this tour didn't take.
    for old in (f"{out}/{name}.png", f"{out}/{name}.txt"):
        if os.path.exists(old):
            os.unlink(old)
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(script, f)
    extra = env or {}
    # Each scene gets a cache of its own, so tabs, carets and staged files one scene leaves
    # aren't restored into the next. Sessions need a host, whose socket needs a short path.
    own_cache = None if "XDG_CACHE_HOME" in extra else tempfile.mkdtemp(prefix="cx", dir="/tmp")
    if own_cache:
        extra = {**extra, "XDG_CACHE_HOME": own_cache}
    said = tempfile.NamedTemporaryFile(suffix=".txt", delete=False)
    said.close()
    env = {
        **os.environ,
        "CELLIX_SCREEN_TEXT": said.name,
        "HOME": HOME,
        "TERM_SIZE": size,
        "TERM_BG": background,
        "TERM_PADDING": "10x24",
        **extra,
    }
    env.pop("CLAUDE_CONFIG_DIR", None)
    run = ["python3", "-I", f"{here}/term.py", f.name, binary, *args]
    # Taken once more if it misses: a loaded machine can lose a scene its place, and one
    # that misses twice is worth a person's look.
    for attempt in ("first", "second"):
        try:
            done = subprocess.run(run, cwd=cwd or ROOT, env=env, timeout=timeout, capture_output=True, text=True)
            why = done.stdout.strip().splitlines()[-1:] if done.returncode else None
        except subprocess.TimeoutExpired:
            why = [f"took more than {timeout} seconds"]
        if why is None:
            break
        print(f"retaking {name}: {' '.join(why)}" if attempt == "first" else f"missed twice: {name}", flush=True)
        if own_cache:
            subprocess.run(["pkill", "-9", "-f", own_cache])
    os.unlink(f.name)
    for left in (said.name, said.name[:-4] + ".new"):
        if os.path.exists(left):
            os.unlink(left)
    if own_cache:
        subprocess.run(["pkill", "-9", "-f", own_cache])
        shutil.rmtree(own_cache, ignore_errors=True)
    if why is None and os.path.exists(f"{out}/{name}.png"):
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
    # New Workspace stands after the four tiles, which keep more room than they did.
    shoot("105-new-workspace-hover", [*told, ["move", 74, 43], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    shoot("106-folder-finder", [*told, ["click", 74, 43], ["wait", 0.6]], CODE_FILE, size="150x44", **space)
    shoot("107-folder-path", [*told, ["click", 74, 43], ["keys", "../in"], ["wait", 0.6]], CODE_FILE,
          size="150x44", **space)
    shoot("101-workspace-hover", [*told, ["move", 40, 43], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    # What waits everywhere, in the side pane: an agent asking, a finished turn, a failed run.
    failed = hook("tool-failed", others["api-server"], tool_input={"command": "cargo test orders"},
                  tool_response={"stdout": "test refunds::rejects_twice ... FAILED\ntest result: FAILED. 13 passed; 1 failed"})
    inbox = [*told, failed, ["wait", 0.6], *SHOW_INBOX, ["wait", 0.8]]
    shoot("140-inbox", inbox, CODE_FILE, size="150x44", **space)
    shoot("141-inbox-hover", [*inbox, ["move", 120, 7], ["wait", 0.4]], CODE_FILE, size="150x44", **space)
    # Find, after a press on the pane, looks through the list in front of it.
    find = [*inbox, ["click", 100, 1], ["wait", 0.3], ["keys", CTRL("f")], ["wait", 0.4],
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


def git_scenes(scratch):
    """Git past staging: a merge stopped on a conflict, stashes, the graph, a file's timeline
    and Compare With, in a project of its own with some history."""
    root = f"{scratch}/history"
    os.makedirs(f"{root}/src")
    def write(name, text):
        with open(f"{root}/{name}", "w") as f:
            f.write(text)
    def commit(message):
        git(root, "add", "-A")
        git(root, "commit", "-qm", message)
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
    shoot("82-git-commit-open", [*graph, ["wait", 3.0], ["click", 14, 2], ["wait", 2.0]], "README.md", size="120x40", cwd=root)
    shoot("83-git-timeline", [["wait", 1.5]], "src/queue.rs", size="120x40", cwd=root)
    compare = [["wait", 1.0], ["keys", "\x1b[112;6u"], ["until", PALETTE_UP], ["keys", "Compare With"],
               ["until", "Compare With"], ["keys", "\r"], ["until", "Compare the file"]]
    shoot("84-git-compare", compare, "src/queue.rs", size="120x40", cwd=root)
    shoot("85-git-compared", [*compare, ["keys", "agent"], ["wait", 0.8], ["keys", "\r"], ["wait", 1.5]],
          "src/queue.rs", size="120x40", cwd=root)
    # Then a merge that stops on a conflict.
    subprocess.run(["git", "-c", "user.name=tour", "-c", "user.email=tour@example.com", "merge", "-q",
                    "agent/queue-bound"], cwd=root, capture_output=True)
    source = [["wait", 1.0], ["click", RAIL_X, REVIEW], ["wait", 3.0]]
    shoot("86-git-merge-panel", source, "src/queue.rs", size="120x40", cwd=root)
    shoot("87-git-conflict", [["wait", 0.8]], "src/queue.rs", size="120x40", cwd=root)
    shoot("88-git-conflict-hover", [["wait", 1.0], ["move", 100, 16], ["wait", 0.6]],
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
        ("64-transcript-tab-hover", [*start, ["move", 104, 0], ["wait", 0.6]], [CODE_FILE], {}),
        ("64-transcript-tab-menu", [*start, ["right", 104, 0], ["wait", 0.4]], [CODE_FILE], {}),
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
    OUTPUT_FROM, OUTPUT_TO, OUTPUT_ASK = (97, 7), (125, 10), (143, 6)
    PROBLEM_COUNT, PROBLEM_ROW = (9, 42), (150, 3)
    GEAR = (RAIL_X, 36)
    settings = [["click", *GEAR], ["until", "Changes apply"]]

    workspaces(scratch)
    code_intelligence(scratch)
    transcripts(scratch)
    task_scenes(scratch)
    shoot("01-welcome", [])
    shoot("02-code", [], CODE_FILE)
    shoot("03-page", [], PAGE_FILE)
    shoot("04-menu-page", [["right", 60, 12]], PAGE_FILE)
    shoot("05-menu-code", [["right", 60, 12]], CODE_FILE)
    shoot("06-menu-explorer", [["right", 12, 2]])
    shoot("07-menu-tab", [["wait", 1.0], ["right", 36, 0], ["wait", 0.3]], CODE_FILE)
    shoot("08-open-file", [["keys", CTRL("p")]], CODE_FILE)
    shoot("09-commands", [["keys", F1], ["until", PALETTE_UP], ["keys", "op"]], CODE_FILE)
    shoot("10-branches", [["wait", 1.5], ["click", 3, 38], ["wait", 0.5]], CODE_FILE)
    shoot("11-find", [["keys", CTRL("f")], ["keys", "item"]], CODE_FILE)
    shoot("12-search", [["click", RAIL_X, SEARCH]], CODE_FILE)
    replacing = [["keys", CTRL("r")], ["keys", "item"], ["keys", "\t"], ["keys", "entry"]]
    shoot("11-replace", replacing, CODE_FILE)
    shoot("12-search-switches", [["click", RAIL_X, SEARCH], ["keys", "item"], ["keys", "\x1bw"], ["wait", 0.6]], CODE_FILE)
    shoot("13-review", [["keys", "\u0007"], ["wait", 1.2]], CODE_FILE)
    shoot("13-strip-hover", [["wait", 0.8], ["move", 100, 0], ["wait", 0.4]], CODE_FILE)
    shoot("13-agents-click", [["wait", 1.5], ["click", 111, 0], ["wait", 1.5]], CODE_FILE)
    shoot("13-quit-hover", [["wait", 6], ["move", 119, 0], ["wait", 1.2]], CODE_FILE)
    shoot("13-quit-beside-pane", [["wait", 6], ["click", 111, 0], ["wait", 3.5], ["move", 116, 0], ["wait", 1.2]], CODE_FILE)
    git_tab = [["click", RAIL_X, REVIEW], ["wait", 0.8]]
    shoot("13-source-hover", [*git_tab, ["move", 14, 11], ["wait", 0.3]], CODE_FILE)
    shoot("13-source-diff", [*git_tab, ["click", 14, 11], ["wait", 1.2]], CODE_FILE)
    shoot("13-source-staged", [*git_tab, ["click", 28, 11], ["wait", 0.8], ["move", 14, 11]], CODE_FILE)
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
                                ["click", 3, 38], ["wait", 0.6]], "src/main.c", cwd=tasks)
    shoot("133-open-recent", palette("open recent"), CODE_FILE)
    to_section = lambda tabs: [*palette("open settings"), ["until", "Changes apply"],
                               *[["keys", "\t"]] * tabs, ["keys", "\r"], ["wait", 0.5]]
    shoot("134-settings-editor", to_section(2), CODE_FILE)
    shoot("135-settings-keyboard", to_section(4), CODE_FILE)
    shoot("136-settings-keys-search", [*to_section(4), ["keys", "task"], ["wait", 0.4]], CODE_FILE)
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
    shoot("30-hover-menu", [["right", 60, 12], ["move", 64, 13]], CODE_FILE)
    shoot("31-hover-palette", [["keys", CTRL("p")], ["move", 60, 7]], CODE_FILE)
    shoot("32-hover-toolbar", [["move", 100, 1]], PAGE_FILE)
    shoot("33-hover-branch", [["move", 3, 38]], CODE_FILE)
    shoot("34-hover-header", [["move", 18, 0]], CODE_FILE)
    shoot("35-hover-review", [["keys", "\u0007"], ["wait", 0.6], ["move", 12, 2]], CODE_FILE)
    shoot("36-hover-accept", [["keys", "\u0007"], ["wait", 0.6], ["move", 114, 2]], CODE_FILE)
    shoot("37-hover-settings", [*settings, ["click", 45, 4], ["move", 80, 21]], CODE_FILE)
    shoot("38-hover-dialog", [["keys", "x"], ["keys", CTRL("q")], ["move", 70, 17]], CODE_FILE)
    shoot("39-hover-agents", [*LIST_AGENTS, ["wait", 0.5], ["move", 90, 8]], CODE_FILE)
    shoot("41-hover-tab-close", [["move", 42, 0]], CODE_FILE)
    shoot("42-hover-install", [*settings, ["click", 45, 4], ["move", 113, 26]], CODE_FILE)
    shoot("43-light-hover-menu", [["right", 60, 12], ["move", 64, 13]], CODE_FILE, background="eff1f5")
    shoot("40-hover-find", [["keys", CTRL("f")], ["keys", "item"], ["move", 112, 3]], CODE_FILE)

    FORMULA, TABLE = (77, 1), (80, 1)
    shoot("44-note", [], NOTE_FILE, size="150x44", background="eff1f5")
    shoot("45-note-dark", [], NOTE_FILE, size="150x44")
    shoot("46-formula-open", [["click", 90, 13], ["wait", 0.5]], NOTE_FILE, size="150x44")
    shoot("47-math-panel", [["click", *FORMULA]], NOTE_FILE, size="150x44", background="eff1f5")
    shoot("48-table-picker", [["click", *TABLE], ["move", 100, 7]], NOTE_FILE, size="150x44")
    # In a window this wide the page stands eight columns in from where its pane's margin is.
    shoot("49-table-menu", [["right", 49, 29]], NOTE_FILE, size="150x44", background="eff1f5")
    # A debug build presents a page of math slowly, so each step is given time to land.
    comment = [["wait", 1.5], ["click", 90, 5], ["wait", 0.6], *command("add comment", F1)]
    comment += [["until", "Enter sends"], ["keys", "Is this true for multi-head attention?\r"]]
    comment += [["gone", "Enter sends"], ["wait", 0.5]]
    shoot("50-comments", comment, NOTE_FILE, size="150x44", background="eff1f5")
    # Being typed: the caret stands after the space just typed.
    typing = [["wait", 3.0], ["click", 90, 5], ["wait", 1.5], *command("add comment", F1),
              ["until", "Enter sends"], ["keys", "Is this true for "], ["wait", 1.5]]
    shoot("50-comment-typing", typing, NOTE_FILE, size="150x44", background="eff1f5")
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
    with open(f"{ROOT}/.cellix/threads.json", "w") as f:
        json.dump(pictured, f)
    with_picture = [["wait", 1.0], *command("show comments", F1), ["until", "yours"], ["wait", 1.5]]
    shoot("50-comment-picture", with_picture, NOTE_FILE, size="150x44")
    shoot("50-comment-picture-light", with_picture, NOTE_FILE, size="150x44", background="eff1f5")
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
    shoot("50-review", comments, NOTE_FILE, size="150x44")
    shoot("50-plan-versions", comments, "docs/PLAN.md", size="150x44")
    # Narrow beside the editor: a pill that can't share its header's row goes under it.
    shoot("50-plan-versions-narrow", comments, "docs/PLAN.md", size="96x44")
    shoot("51-agent-list", [*LIST_AGENTS, ["wait", 1.0]], NOTE_FILE, size="150x44")
    shoot("51-inbox-empty", [*SHOW_INBOX, ["wait", 0.6]], NOTE_FILE, size="150x44")
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
    shoot("57-dock-comments", [["wait", 1.5], *command("show comments", F1), ["wait", 1.0]], NOTE_FILE,
          size="64x40")
    shoot("57-dock-agents", [*LIST_AGENTS, ["wait", 0.6]], NOTE_FILE, size="64x40")
    shoot("57-dock-menu", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4]], NOTE_FILE, size="64x40")
    # Format is the fourth item down, past the rule under Paste: menus stand on whole rows.
    shoot("57-dock-menu-format", [["wait", 1.5], ["right", 20, 12], ["wait", 0.4], ["move", 24, 17],
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
    start_shell = [*command("New Terminal"), ["wait", 1.5]]
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

    # A window wider than a page is set: the page keeps to its reading width, in the middle,
    # and what's wider than that moves sideways inside it.
    WIDE_FILE = "docs/wide.md"
    with open(f"{ROOT}/{WIDE_FILE}", "w") as f:
        f.write(WIDE)
    wide = {"size": "200x50"}
    shoot("160-reading-width", [["wait", 1.0]], WIDE_FILE, **wide)
    shoot("161-reading-width-across", [["wait", 1.0], ["tip", 100, 21, 6], ["tip", 100, 30, 4],
                                       ["wait", 0.5]], WIDE_FILE, **wide)
    shoot("162-reading-width-note", [["wait", 1.0]], NOTE_FILE, background="eff1f5", **wide)
    shoot("163-reading-width-paper", [["wait", 1.0]], PAPER, **wide)
    shoot("164-reading-width-sample", [["wait", 1.0], ["wheel", 100, 20, 9], ["wait", 1.0]], WRITING, **wide)
    # A narrow side pane with more tabs than it has room for: the one in front whole, the
    # others cut short, and one of those under the pointer.
    three_tabs = [*LIST_AGENTS, *command("show comments", F1), ["until", "No comments"],
                  *command("New Terminal"), ["wait", 1.5]]
    shoot("165-pane-tabs-cut", three_tabs, CODE_FILE)
    shoot("166-pane-tabs-cut-hover", [*three_tabs, ["move", 83, 0], ["wait", 1.2]], CODE_FILE)
    shoot("167-pane-tabs-cut-light", [*three_tabs, ["click", 83, 0], ["wait", 0.6]], CODE_FILE,
          background="eff1f5")

    # The code view: fold chevrons and chips, sticky scroll, word wrap, bracket pairs. The
    # file opens with the gutter's chevrons in columns 37 and 38, its line n on row n + 1.
    VIEW_FILE = "src/scheduler.rs"
    with open(f"{ROOT}/{VIEW_FILE}", "w") as f:
        f.write(SCHEDULER)
    FOLD_KEYS = "\x1b[91;6u"
    at_line = lambda n: n + 1
    shoot("110-view-fold-hover", [["move", 37, at_line(16)], ["wait", 0.4]], VIEW_FILE)
    # run_due folded first, then new above it, which leaves run_due's row where it was.
    folding = [["click", 60, at_line(16)], ["keys", FOLD_KEYS], ["click", 60, at_line(12)],
               ["keys", FOLD_KEYS], ["wait", 0.3]]
    shoot("111-view-folded", folding, VIEW_FILE)
    shoot("111-view-folded-hover", [*folding, ["move", 37, at_line(11)], ["wait", 0.4]], VIEW_FILE)
    short = {"size": "120x24"}
    shoot("112-view-sticky", [["wheel", 70, 12, 16], ["wait", 0.8]], VIEW_FILE, **short)
    # Each scene opens the file where the last one left it, so these go to the top first.
    home = ["keys", "\x1b[1;5H"]
    shoot("113-view-wrap", [home, ["click", 60, at_line(5)], ["keys", "\x1bz"], ["wait", 0.4]], VIEW_FILE)
    shoot("114-view-brackets", [home, ["click", 59, at_line(16)], ["wait", 0.3]], VIEW_FILE)
    shoot("115-view-light", [["wheel", 70, 12, 16], ["wait", 0.8], ["move", 37, 14], ["wait", 0.4]],
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
    git_scenes(scratch)
    # Agent Context: a project with instructions that disagree and an import, a user's own
    # instructions and skill, and a session's transcript, all in the throwaway home.
    CONTEXT = 6  # the fifth rail icon's middle
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
    show_context = [["click", RAIL_X, CONTEXT], ["wait", 1.2]]
    shoot("81-context", show_context, cwd=ctx, size="150x44")
    shoot("81-context-hover", [*show_context, ["move", 14, 9], ["wait", 0.3]], cwd=ctx, size="150x44")
    # Checks wrap to however many rows their words take, so fold them to find AGENTS.md's row.
    fold_checks = [["click", 14, 6], ["wait", 0.3]]
    shoot("81-context-open", [*show_context, *fold_checks, ["click", 14, 11], ["wait", 1.0]], cwd=ctx, size="150x44")
    shoot("81-context-codex", [*show_context, ["click", 14, 1], ["wait", 1.2]], cwd=ctx, size="150x44")

if FAILED:
    print(f"{len(FAILED)} of {len(SHOT) + len(FAILED)} scenes failed: {', '.join(FAILED)}", flush=True)
    sys.exit(1)
