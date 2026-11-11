#!/usr/bin/env python3
"""A headless terminal for the real binary: it answers Cellix's questions, keeps the Kitty
graphics a terminal would keep, and writes what's on screen as a PNG. No window, no focus,
and unlike `harness/x snapshot` it goes through the tile cache and smooth scrolling, so it
sees what a person sees.

    term.py <script.json> <command...>

The script is a list of steps:
    ["wait", seconds]            ["keys", "text, with \\e as ESC"]
    ["click", col, row]          ["right", col, row]       ["middle", col, row]
    ["double", col, row]         ["triple", col, row]      ["drag", col, row, col, row]
    ["press", col, row]          ["hold", col, row]        A press, and a move with it still held.
    ["move", col, row]           ["wheel", col, row, notches]  (negative is up)
    ["paste", "text"]            ["resize", cols, rows]
    ["shot", "out.png"]          ["log", "label"]
    ["until", "text"]            Waits until the screen says it, and fails the script if it never does.
    ["gone", "text"]             Waits until the screen no longer says it.
    ["until", "text", seconds]   Either, giving up after that long rather than 20 seconds.
    ["timed", "label", <step>]   Runs a step, and logs how long until the screen answered.

Settings come from the environment: TERM_SIZE=170x46, TERM_CELL=17x36, TERM_PADDING=10x8,
TERM_BG=1e1e2e. Clipboard writes and timings are printed. Stop when the script ends.

The terminal is sent pictures, so it can't read the screen. With CELLIX_SCREEN_TEXT set to a
path, Cellix writes the screen's text there on every frame: "until" reads it, and each shot
leaves that text beside its PNG. A script whose "until" never comes takes no more shots and
exits with 3, so a picture of the wrong surface is never written."""
import subprocess, base64, fcntl, json, os, pty, re, select, signal, struct, sys, termios, time, zlib


def pair(name, default):
    a, b = os.environ.get(name, default).split("x")
    return int(a), int(b)


COLS, ROWS = pair("TERM_SIZE", "170x46")
CW, CH = pair("TERM_CELL", "17x36")
PAD_X, PAD_Y = pair("TERM_PADDING", "10x8")
BG = bytes.fromhex(os.environ.get("TERM_BG", "1e1e2e"))
TEXT = os.environ.get("CELLIX_SCREEN_TEXT")


def said():
    """What Cellix says is on screen, as text. Empty before its first frame."""
    try:
        with open(TEXT, encoding="utf8", errors="replace") as f:
            return f.read()
    except (OSError, TypeError):
        return ""
CHANNEL = lambda byte: f"{byte:02x}{byte:02x}"
BACKGROUND = "/".join(CHANNEL(b) for b in BG)


class Screen:
    """The part of a terminal Cellix's pixel tiers use."""

    def __init__(self):
        self.images = {}  # id -> (width, height, rgb)
        self.partial = {}
        self.placements = {}  # (id, number) -> placement
        self.order = 0
        self.row, self.col = 0, 0
        self.pending = None  # the control of a chunked transmit
        self.clipboard = []
        self.replies = []
        self.buffer = b""
        self.frames = 0

    def feed(self, data):
        self.buffer += data
        while self.buffer:
            start = self.buffer.find(b"\x1b")
            if start < 0:
                self.buffer = b""
                return
            self.buffer = self.buffer[start:]
            used = self.escape(self.buffer)
            if used == 0:
                return  # incomplete, wait for more
            self.buffer = self.buffer[used:]

    def escape(self, b):
        if len(b) < 2:
            return 0
        kind = b[1:2]
        if kind == b"_":
            end = b.find(b"\x1b\\")
            if end < 0:
                return 0
            self.graphics(b[3:end])
            return end + 2
        if kind == b"]":
            end = min((i for i in (b.find(b"\x07"), b.find(b"\x1b\\")) if i >= 0), default=-1)
            if end < 0:
                return 0
            self.osc(b[2:end])
            return end + (1 if b[end:end + 1] == b"\x07" else 2)
        if kind == b"[":
            match = re.match(rb"\x1b\[([?<>=]?)([0-9;:]*)([$ ]?)([@-~])", b)
            if not match:
                return 0 if len(b) < 64 else 2
            self.csi(*match.groups())
            return match.end()
        return 2

    def csi(self, private, args, middle, final):
        if final == b"H":
            parts = [int(p or 1) for p in args.split(b";")] if args else [1, 1]
            self.row, self.col = parts[0] - 1, (parts[1] if len(parts) > 1 else 1) - 1
        elif final == b"t" and args == b"16":
            self.replies.append(f"\x1b[6;{CH};{CW}t")
        elif final == b"c" and not private:
            self.replies.append("\x1b[?62;22c")
        elif final == b"p" and middle == b"$" and args == b"1016":
            self.replies.append("\x1b[?1016;2$y")
        elif final == b"l" and args == b"2026":
            self.frames += 1
        elif final == b"J" and args == b"2":
            pass

    def osc(self, body):
        if body == b"11;?":
            self.replies.append(f"\x1b]11;rgb:{BACKGROUND}\x1b\\")
        elif body.startswith(b"52;c;"):
            text = base64.b64decode(body[5:]).decode("utf8", "replace")
            self.clipboard.append(text)
            print(f"clipboard: {text!r}", flush=True)

    def graphics(self, body):
        control, _, payload = body.partition(b";")
        keys = dict(p.split("=", 1) for p in control.decode().split(",") if "=" in p)
        action = keys.get("a", "t")
        if self.pending is not None and "a" not in keys:
            keys, action = {**self.pending, "m": keys.get("m", "0")}, "t"
        if action == "q":
            self.replies.append(f"\x1b_Gi={keys['i']};OK\x1b\\")
        elif action == "t":
            image = int(keys["i"])
            self.partial[image] = self.partial.get(image, b"") + payload
            if keys.get("m") == "1":
                self.pending = keys
                return
            self.pending = None
            data = zlib.decompress(base64.b64decode(self.partial.pop(image)))
            self.images[image] = (int(keys["s"]), int(keys["v"]), data)
        elif action == "p":
            self.order += 1
            image, number = int(keys["i"]), int(keys.get("p", 0))
            get = lambda k: int(keys.get(k, 0))
            self.placements[(image, number)] = dict(
                order=self.order, row=self.row, col=self.col, z=get("z"), y_off=get("Y"),
                sx=get("x"), sy=get("y"), sw=get("w"), sh=get("h"),
            )
        elif action == "d":
            what = keys.get("d", "a")
            if what == "A":
                self.images.clear()
                self.placements.clear()
            elif what == "I":
                self.images.pop(int(keys["i"]), None)
                self.drop(lambda k, p: k[0] == int(keys["i"]))
            elif what == "i":
                image, number = int(keys["i"]), int(keys.get("p", 0))
                self.drop(lambda k, p: k[0] == image and (not number or k[1] == number))
            elif what == "z":
                self.drop(lambda k, p: p["z"] == int(keys["z"]))

    def drop(self, test):
        for key in [k for k, p in self.placements.items() if test(k, p)]:
            del self.placements[key]

    def picture(self):
        # The whole window: Ghostty lets an image run over its right and bottom padding.
        width, height = COLS * CW + PAD_X * 2, ROWS * CH + PAD_Y * 2
        canvas = bytearray(BG * (width * height))
        for (image, _), p in sorted(self.placements.items(), key=lambda i: (i[1]["z"], i[1]["order"])):
            if image not in self.images:
                continue
            iw, ih, rgb = self.images[image]
            sw, sh = p["sw"] or iw, p["sh"] or ih
            left, top = PAD_X + p["col"] * CW, PAD_Y + p["row"] * CH + p["y_off"]
            for line in range(sh):
                y = top + line
                if not 0 <= y < height:
                    continue
                source = ((p["sy"] + line) * iw + p["sx"]) * 3
                run = min(sw, width - left)
                canvas[(y * width + left) * 3:(y * width + left + run) * 3] = rgb[source:source + run * 3]
        return width, height, bytes(canvas)


def png(path, width, height, rgb):
    rows = [rgb[y * width * 3:(y + 1) * width * 3] for y in range(height)]
    raw = b"".join(b"\0" + r for r in rows)

    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header))
        f.write(chunk(b"IDAT", zlib.compress(raw, 3)) + chunk(b"IEND", b""))


def mouse(button, col, row, press=True):
    # Mode 1016: pixels, in the middle of the cell.
    x, y = int((col + 0.5) * CW) + 1, int((row + 0.5) * CH) + 1
    return f"\x1b[<{button};{x};{y}{'M' if press else 'm'}".encode()


def main():
    script = json.load(open(sys.argv[1]))
    pid, master = pty.fork()
    if pid == 0:
        os.environ["TERM"] = "xterm-ghostty"
        os.environ["TERM_PROGRAM"] = "ghostty"
        os.execvp(sys.argv[2], sys.argv[2:])

    def set_size(cols, rows):
        size = struct.pack("HHHH", rows, cols, cols * CW, rows * CH)
        fcntl.ioctl(master, termios.TIOCSWINSZ, size)

    set_size(COLS, ROWS)
    screen = Screen()
    alive = [True]
    received = [0]
    missed = []

    def pump(seconds):
        """Runs the terminal for a while. Returns seconds until the first frame, if any."""
        deadline = time.time() + seconds
        started, first = time.time(), None
        frames = screen.frames
        while True:
            left = deadline - time.time()
            if left <= 0:
                return first
            ready, _, _ = select.select([master], [], [], min(left, 0.02))
            if master in ready:
                try:
                    data = os.read(master, 1 << 20)
                except OSError:
                    data = b""
                if not data:
                    alive[0] = False
                    return first
                if b"panicked" in data:
                    text = re.sub(rb"\x1b[\[_\]][^\x1b]*(\x1b\\\\)?", b"", data)
                    print(text.decode("utf8", "replace")[-600:], flush=True)
                received[0] += len(data)
                screen.feed(data)
            for reply in screen.replies:
                os.write(master, reply.encode())
            screen.replies.clear()
            if first is None and screen.frames != frames and has_content(screen):
                first = time.time() - started

    def has_content(_):
        return True

    def run(step):
        kind = step[0]
        if kind == "wait":
            pump(step[1])
        elif kind in ("until", "gone"):
            deadline = time.time() + (step[2] if len(step) > 2 else 20)
            while (step[1] in said()) == (kind == "gone") and alive[0]:
                if time.time() > deadline or not TEXT:
                    missed.append(("never showed " if kind == "until" else "still showed ") + repr(step[1]))
                    return
                pump(0.05)
            # The frame that says it is on its way to the terminal as well.
            pump(0.1)
        elif kind == "keys":
            os.write(master, step[1].replace("\\e", "\x1b").encode())
            pump(0.25)
        elif kind == "paste":
            os.write(master, f"\x1b[200~{step[1]}\x1b[201~".encode())
            pump(0.25)
        elif kind in ("click", "right", "middle"):
            button = {"click": 0, "middle": 1, "right": 2}[kind]
            os.write(master, mouse(button, step[1], step[2]))
            pump(0.02)
            os.write(master, mouse(button, step[1], step[2], False))
            pump(0.25)
        elif kind in ("double", "triple"):
            for _ in range(2 if kind == "double" else 3):
                os.write(master, mouse(0, step[1], step[2]))
                pump(0.02)
                os.write(master, mouse(0, step[1], step[2], False))
                pump(0.03)
            pump(0.25)
        elif kind == "drag":
            os.write(master, mouse(0, step[1], step[2]))
            pump(0.03)
            os.write(master, mouse(32, step[3], step[4]))
            pump(0.03)
            os.write(master, mouse(0, step[3], step[4], False))
            pump(0.25)
        elif kind == "press":
            os.write(master, mouse(0, step[1], step[2]))
            pump(0.1)
        elif kind == "hold":
            os.write(master, mouse(32, step[1], step[2]))
            pump(0.25)
        elif kind == "move":
            os.write(master, mouse(35, step[1], step[2]))
            pump(0.2)
        elif kind == "wheel":
            button = 65 if step[3] > 0 else 64
            for _ in range(abs(step[3])):
                os.write(master, mouse(button, step[1], step[2]))
                pump(0.09)
            pump(0.25)
        elif kind == "resize":
            set_size(step[1], step[2])
            pump(0.4)
        elif kind == "shot":
            # A frame is prepared whole and then written, so a slow build looks quiet
            # while it's still drawing. Half a change is nobody's screen.
            quiet = time.time()
            start = time.time()
            while time.time() - quiet < 1.2 and time.time() - start < 10 and alive[0]:
                before = received[0]
                pump(0.02)
                if received[0] - before > 256:
                    quiet = time.time()
            png(step[1], *screen.picture())
            if TEXT:
                with open(os.path.splitext(step[1])[0] + ".txt", "w", encoding="utf8") as f:
                    f.write(said())
        elif kind == "log":
            print(step[1], flush=True)
        elif kind == "bg":
            # The terminal turns light or dark, as a Mac does at sunset, and the window comes
            # back into focus.
            global BACKGROUND
            os.write(master, b"\x1b[O")
            pump(0.3)
            BACKGROUND = "/".join(CHANNEL(b) for b in bytes.fromhex(step[1]))
            os.write(master, b"\x1b[I")
            pump(0.5)
        elif kind == "run":
            # A command beside the program, as an agent's hook or a restart would come.
            subprocess.run(step[1], shell=True, input=step[2] if len(step) > 2 else None,
                           text=True, capture_output=True, timeout=20)
            pump(0.4)
        elif kind == "timed":
            sent = time.time()
            frames = screen.frames
            inner = step[2]
            if inner[0] == "keys":
                os.write(master, inner[1].replace("\\e", "\x1b").encode())
            else:
                button = {"click": 0, "right": 2}[inner[0]]
                os.write(master, mouse(button, inner[1], inner[2]))
                os.write(master, mouse(button, inner[1], inner[2], False))
            while screen.frames == frames and time.time() - sent < 5 and alive[0]:
                pump(0.002)
            print(f"{step[1]}: {(time.time() - sent) * 1000:.0f} ms", flush=True)
            pump(0.2)

    # A cold start can take seconds, so wait for a real frame rather than guessing.
    deadline = time.time() + 20
    while not screen.placements and alive[0] and time.time() < deadline:
        pump(0.05)
    pump(0.4)
    # And with the screen told as text, until it has said its first frame.
    while TEXT and not said().strip() and alive[0] and time.time() < deadline:
        pump(0.05)
    for step in script:
        if not alive[0]:
            print("the program exited early", flush=True)
            break
        run(step)
        if missed:
            shown = " / ".join(line.strip() for line in said().splitlines() if line.strip())
            print(f"{missed[0]}; the screen says: {shown[:400]}", flush=True)
            break
    try:
        os.kill(pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    # The next scene may need what this one holds, such as a workspace's socket.
    deadline = time.time() + 5
    while time.time() < deadline:
        try:
            done, _ = os.waitpid(pid, os.WNOHANG)
        except ChildProcessError:
            break
        if done:
            break
        time.sleep(0.05)
    if missed:
        sys.exit(3)


main()
