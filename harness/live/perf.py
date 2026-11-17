#!/usr/bin/env python3
"""What the real binary costs while nothing happens, while a shell in the side pane prints,
and while shells in tabs behind it print: its CPU, what it sends the terminal, and, where it
keeps them (`CELLIX_PROF`), its own counts of wakes, frames and tiles. Run by `harness/x perf`,
through the guard, against the last release when there is one.

    perf.py <candidate> [<baseline>]

Each binary runs in a headless terminal of its own, as the tour's does: it's answered as
Ghostty answers, sent no window, and given a throwaway home, cache and project, so nothing
of the person's is read or written. A case is set up, its writers are let go, it warms up for
PERF_WARMUP seconds (3) and is measured for PERF_SECONDS (10). Two rounds, the second in the
other order, so a machine getting busier halfway doesn't favour one binary. A writer that
printed less than most of what it should is a failed run: back-pressure that slowed the
program would otherwise read as a saving.

    idle      a file open, nothing happening: no frames, no bytes
    visible   a shell in the side pane printing a line in place 30 times a second
    hidden    four shells in tabs behind the one in front, each printing 60 times a second

PERF_CASES=idle,visible picks cases, PERF_ROUNDS how many rounds. The table is printed and kept as JSON in PERF_OUT
(target/perf). It fails if the candidate takes more than a quarter more CPU than the
baseline and over half a point more, in any case."""
import fcntl, json, os, pty, re, select, shutil, signal, struct, subprocess, sys, tempfile, termios, time

HERE = os.path.dirname(os.path.abspath(__file__))
PRODUCER = os.path.join(HERE, "perf_producer.py")
COLS, ROWS, CW, CH = 170, 46, 17, 36
SECONDS = float(os.environ.get("PERF_SECONDS", "10"))
WARMUP = float(os.environ.get("PERF_WARMUP", "3"))
CASES = os.environ.get("PERF_CASES", "idle,visible,hidden").split(",")
ROUNDS = int(os.environ.get("PERF_ROUNDS", "2"))
OUT = os.environ.get("PERF_OUT", "target/perf")
CTRL_J = b"\n"
PALETTE = b"\x1b[112;6u"


class Terminal:
    """The binary on a PTY, answered as a terminal answers, its output counted."""

    def __init__(self, binary, args, env, cwd):
        self.pid, self.master = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execve(binary, [binary, *args], env)
        size = struct.pack("HHHH", ROWS, COLS, COLS * CW, ROWS * CH)
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, size)
        self.text = env["CELLIX_SCREEN_TEXT"]
        self.tail = b""
        self.bytes = 0
        self.frames = 0
        self.alive = True

    def answer(self, data):
        seen = self.tail + data
        replies = []
        if b"\x1b[16t" in seen:
            replies.append(f"\x1b[6;{CH};{CW}t")
        if b"\x1b[?1016$p" in seen:
            replies.append("\x1b[?1016;2$y")
        if b"\x1b]11;?" in seen:
            replies.append("\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\")
        for query in re.findall(rb"\x1b_Gi=(\d+),[^;\x1b]*a=q", seen):
            replies.append(f"\x1b_Gi={query.decode()};OK\x1b\\")
        if re.search(rb"\x1b\[c|\x1b\[0c", seen):
            replies.append("\x1b[?62;22c")
        self.tail = seen[-64:] if not replies else b""
        for reply in replies:
            os.write(self.master, reply.encode())

    def pump(self, seconds):
        deadline = time.time() + seconds
        while self.alive:
            left = deadline - time.time()
            if left <= 0:
                return
            ready, _, _ = select.select([self.master], [], [], min(left, 0.05))
            if not ready:
                continue
            try:
                data = os.read(self.master, 1 << 20)
            except OSError:
                data = b""
            if not data:
                self.alive = False
                return
            self.bytes += len(data)
            self.frames += data.count(b"\x1b[?2026l")
            # Questions come at the start and on focus; replies are only looked for while
            # what comes is small, which a frame of tiles isn't.
            if len(data) < 4096:
                self.answer(data)

    def said(self):
        try:
            with open(self.text, encoding="utf8", errors="replace") as f:
                return f.read()
        except OSError:
            return ""

    def until(self, test, seconds=20):
        deadline = time.time() + seconds
        while time.time() < deadline and self.alive:
            if test(self.said()):
                self.pump(0.2)
                return True
            self.pump(0.05)
        return False

    def keys(self, data):
        os.write(self.master, data)
        self.pump(0.2)

    def stop(self):
        try:
            os.kill(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
        deadline = time.time() + 5
        while time.time() < deadline:
            self.pump(0.05)
            try:
                done, _ = os.waitpid(self.pid, os.WNOHANG)
            except ChildProcessError:
                return
            if done:
                return
        os.kill(self.pid, signal.SIGKILL)


def cpu_seconds(pids):
    """CPU time used so far by each of `pids` that's still there."""
    if not pids:
        return {}
    out = subprocess.run(["ps", "-o", "pid=,time=", "-p", ",".join(map(str, pids))],
                         capture_output=True, text=True).stdout
    used = {}
    for line in out.splitlines():
        pid, clock = line.split()
        parts = [float(p) for p in clock.split(":")]
        seconds = 0.0
        for part in parts:
            seconds = seconds * 60 + part
        used[int(pid)] = seconds
    return used


def hosts(cache):
    """The session hosts of a run, which hold its shells: their command names its cache."""
    out = subprocess.run(["pgrep", "-f", f"session-host {cache}"], capture_output=True, text=True).stdout
    return [int(p) for p in out.split()]


def counts(path, start, end):
    """The counters' lines written between two offsets of their file, summed."""
    totals = {}
    try:
        with open(path, "rb") as f:
            f.seek(start)
            chunk = f.read(end - start).decode("utf8", "replace")
    except OSError:
        return None
    # Whole lines only: the last may still be being written.
    lines = [line for line in chunk.split("\n")[:-1] if line.startswith("t=") and "present_ms=" in line]
    if not lines:
        return None
    for line in lines:
        for word in line.split():
            key, _, value = word.partition("=")
            if "/" in value or key == "t":
                continue
            totals[key] = totals.get(key, 0) + int(value)
    return totals


def run_case(binary, case, scratch):
    home = tempfile.mkdtemp(prefix="home", dir=scratch)
    # Sessions need a host, whose socket needs a short path.
    cache = tempfile.mkdtemp(prefix="cx", dir="/tmp")
    project = os.path.join(scratch, f"project-{case}")
    os.makedirs(project, exist_ok=True)
    with open(os.path.join(project, "notes.md"), "w") as f:
        f.write("# Notes\n\n" + "".join(f"Line {n} of a quiet file.\n" for n in range(60)))
    said_dir = tempfile.mkdtemp(prefix="said", dir=scratch)
    prof = os.path.join(scratch, f"prof-{case}-{os.getpid()}-{time.time_ns()}.log")
    gate = os.path.join(scratch, f"gate-{time.time_ns()}")
    env = {
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": home,
        "XDG_CACHE_HOME": cache,
        "TERM": "xterm-ghostty",
        "TERM_PROGRAM": "ghostty",
        "SHELL": "/bin/sh",
        "PS1": "$ ",
        "ENV": "",
        "LANG": "en_US.UTF-8",
        "CELLIX_MACHINE": "dev-box",
        "CELLIX_SCREEN_TEXT": os.path.join(said_dir, "screen.txt"),
        "CELLIX_PROF": prof,
    }
    term = Terminal(binary, ["notes.md"], env, project)
    writers = []
    try:
        if not term.until(lambda text: "notes.md" in text):
            raise RuntimeError("never drew its first frame")
        command = lambda label, rate: f"{sys.executable} -I {PRODUCER} {rate} {gate} {label}\r".encode()

        def answers(n):
            # The shell is there once it runs something: what it prints isn't what was typed.
            for _ in range(3):
                term.keys(f"printf 'ready%s\\n' {n}\r".encode())
                if term.until(lambda text: f"ready{n}" in text, 8):
                    return True
            return False

        if case == "visible":
            term.keys(CTRL_J)
            if not answers(1):
                raise RuntimeError("no shell in the side pane")
            term.keys(command("v1", 30))
            writers.append(("v1", 30))
        elif case == "hidden":
            for index in range(5):
                if index == 0:
                    term.keys(CTRL_J)
                else:
                    term.keys(PALETTE)
                    term.until(lambda text: "Type to search" in text)
                    term.keys(b"New Terminal")
                    term.until(lambda text: "New Terminal" in text)
                    term.keys(b"\r")
                    # The new tab is in front once what the last one was told is gone.
                    if not term.until(lambda text: "perf_producer" not in text):
                        raise RuntimeError("no new terminal tab")
                if not answers(index + 1):
                    raise RuntimeError("no shell in a new terminal")
                # The last tab, in front, stays at its prompt.
                if index < 4:
                    label = f"h{index + 1}"
                    term.keys(command(label, 60))
                    writers.append((label, 60))
        open(gate, "w").close()
        if case == "visible" and not term.until(lambda text: "bench-output" in text):
            raise RuntimeError("the writer printed nothing")
        # The screen's text is only for setting up: from here, writing it fails at once, and
        # costs both binaries the same.
        os.chmod(said_dir, 0o500)
        term.pump(WARMUP)
        pids = [term.pid, *hosts(cache)]
        before = cpu_seconds(pids)
        written = {label: count_of(gate, label) for label, _ in writers}
        mark = os.path.getsize(prof) if os.path.exists(prof) else 0
        bytes0, frames0 = term.bytes, term.frames
        began = time.time()
        term.pump(SECONDS)
        took = time.time() - began
        after = cpu_seconds(pids)
        end = os.path.getsize(prof) if os.path.exists(prof) else 0
        for label, rate in writers:
            printed = count_of(gate, label) - written[label]
            if printed < rate * SECONDS * 0.7:
                raise RuntimeError(f"writer {label} printed {printed} of about {rate * SECONDS:.0f}")
        main = (after.get(term.pid, 0) - before.get(term.pid, 0)) / took * 100
        held = sum(after.get(p, 0) - before.get(p, 0) for p in pids[1:]) / took * 100
        result = {
            "cpu": round(main, 2),
            "hosts_cpu": round(held, 2),
            "bytes_per_s": round((term.bytes - bytes0) / took),
            "frames_per_s": round((term.frames - frames0) / took, 1),
        }
        kept = counts(prof, mark, end)
        if kept and kept.get("ms"):
            # Per second of what the lines cover, which is whole seconds inside the sample.
            seconds = kept.pop("ms") / 1000
            result["prof"] = {key: round(value / seconds, 1) for key, value in kept.items()}
        return result
    finally:
        try:
            os.chmod(said_dir, 0o700)
        except OSError:
            pass
        term.stop()
        # Sessions outlive Cellix: their hosts go, and the writers with them.
        subprocess.run(["pkill", "-9", "-f", cache])
        subprocess.run(["pkill", "-9", "-f", gate])
        shutil.rmtree(cache, ignore_errors=True)


def count_of(gate, label):
    try:
        with open(f"{gate}.{label}") as f:
            return int(f.read() or 0)
    except (OSError, ValueError):
        return 0


def main():
    candidate = os.path.abspath(sys.argv[1])
    baseline = os.path.abspath(sys.argv[2]) if len(sys.argv) > 2 and sys.argv[2] else None
    variants = {"candidate": candidate, **({"baseline": baseline} if baseline else {})}
    scratch = tempfile.mkdtemp(prefix="cellix-perf-")
    results = {name: {case: [] for case in CASES} for name in variants}
    failed = []
    try:
        for round_ in range(1, ROUNDS + 1):
            order = list(variants)
            if round_ % 2 == 0:
                order.reverse()
            for case in CASES:
                for name in order:
                    try:
                        found = run_case(variants[name], case, scratch)
                    except RuntimeError as e:
                        failed.append(f"{name} {case} round {round_}: {e}")
                        print(f"FAILED {name} {case} round {round_}: {e}", flush=True)
                        continue
                    results[name][case].append(found)
                    print(f"{name} {case} round {round_}: {json.dumps(found)}", flush=True)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)

    def mean(runs, key, inside=None):
        values = [(run.get(inside) or {}).get(key) if inside else run.get(key) for run in runs]
        values = [v for v in values if v is not None]
        return round(sum(values) / len(values), 2) if values else None

    print(f"\nperf: {SECONDS:.0f} s samples after {WARMUP:.0f} s warm-up, {ROUNDS} rounds, {COLS}x{ROWS}")
    head = f"{'case':<8} {'binary':<10} {'cpu %':>7} {'hosts %':>8} {'bytes/s':>9} {'frames/s':>9}"
    head += f" {'wakes/s':>8} {'draws/s':>8} {'sent/s':>7} {'tiles/s':>8} {'rows/s':>7}"
    print(head)
    summary = {}
    for case in CASES:
        for name in variants:
            runs = results[name][case]
            row = {key: mean(runs, key) for key in ("cpu", "hosts_cpu", "bytes_per_s", "frames_per_s")}
            for key in ("wakes", "draws", "sent", "tiles", "rows", "input", "output", "tick", "poll",
                        "resize", "scroll", "hidden", "presents"):
                row[key] = mean(runs, key, "prof")
            summary.setdefault(case, {})[name] = row
            show = lambda v: "-" if v is None else f"{v:g}"
            print(f"{case:<8} {name:<10} {show(row['cpu']):>7} {show(row['hosts_cpu']):>8} "
                  f"{show(row['bytes_per_s']):>9} {show(row['frames_per_s']):>9} {show(row['wakes']):>8} "
                  f"{show(row['draws']):>8} {show(row['sent']):>7} {show(row['tiles']):>8} {show(row['rows']):>7}")
    os.makedirs(OUT, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    with open(os.path.join(OUT, f"perf-{stamp}.json"), "w") as f:
        json.dump({"variants": variants, "seconds": SECONDS, "runs": results, "summary": summary}, f, indent=1)
    if baseline:
        for case in CASES:
            before, after = summary[case]["baseline"]["cpu"], summary[case]["candidate"]["cpu"]
            if before is not None and after is not None and after > before * 1.25 and after - before > 0.5:
                failed.append(f"{case}: the candidate takes {after}% CPU where the baseline took {before}%")
    if failed:
        print("\n" + "\n".join(failed))
        sys.exit(1)


main()
