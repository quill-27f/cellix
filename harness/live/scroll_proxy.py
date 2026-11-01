#!/usr/bin/env python3
"""Runs Cellix in a pty, injects wheel notches, and can freeze output mid-animation, so a
screenshot of the real terminal catches a frame that normally lasts milliseconds."""
import fcntl, os, pty, select, signal, struct, sys, termios, time, tty

freeze_at, marker, *command = sys.argv[1:]
freeze_at = int(freeze_at)
END_FRAME = b"\x1b[?2026l"
# In pixels, because Cellix turns on mode 1016 where the terminal offers it.
WHEEL_DOWN = b"\x1b[<65;1100;550M"

pid, master = pty.fork()
if pid == 0:
    os.execvp(command[0], command)
fcntl.ioctl(master, termios.TIOCSWINSZ, fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8))
tty.setraw(0)
start = time.time()
log = open(marker + ".log", "w")
sent, frames, held, pending = False, 0, None, b""
while True:
    ready, _, _ = select.select([master, 0], [], [], 0.02)
    if not sent and time.time() - start > 4:
        sent = True
        frames = 0
        log.write(f"{time.time() - start:.2f} wheel\n")
        os.write(master, WHEEL_DOWN)
    if held is not None and time.time() > held:
        held = None
        os.write(1, pending)
    if held is None and 0 in ready:
        incoming = os.read(0, 65536)
        if not incoming:
            break
        log.write(f"{time.time() - start:.2f} in {incoming!r}\n")
        log.flush()
        os.write(master, incoming)
    if held is None and master in ready:
        try:
            data = os.read(master, 65536)
        except OSError:
            break
        if sent and freeze_at and frames < freeze_at:
            before = 0
            while True:
                at = data.find(END_FRAME, before)
                if at < 0:
                    break
                frames += 1
                before = at + len(END_FRAME)
                if frames == freeze_at:
                    os.write(1, data[:before])
                    data = data[before:]
                    open(marker, "w").write("frozen")
                    held = time.time() + 6
                    break
        if held is None:
            os.write(1, data)
        else:
            pending = data
