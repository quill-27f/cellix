#!/usr/bin/env python3
"""Runs a command in a pty and feeds it bytes on a schedule, for driving a real terminal
program that nothing else can reach. Usage: pty_inject.py <schedule.json> <command...>
The schedule is a list of [seconds, text], with \\e written as ESC."""
import fcntl, json, os, pty, select, sys, termios, time, tty

schedule = json.load(open(sys.argv[1]))


def size():
    return fcntl.ioctl(0, termios.TIOCGWINSZ, b"\0" * 8)


time.sleep(1)  # The window reports its real size a moment after it opens.
pid, master = pty.fork()
if pid == 0:
    time.sleep(0.2)
    os.execvp(sys.argv[2], sys.argv[2:])
known = size()
fcntl.ioctl(master, termios.TIOCSWINSZ, known)
tty.setraw(0)
start = time.time()
while True:
    ready, _, _ = select.select([master, 0], [], [], 0.02)
    if size() != known:
        known = size()
        fcntl.ioctl(master, termios.TIOCSWINSZ, known)
    while schedule and time.time() - start > schedule[0][0]:
        _, text = schedule.pop(0)
        os.write(master, text.replace("\\e", "\x1b").encode())
    if 0 in ready:
        data = os.read(0, 65536)
        if not data:
            break
        os.write(master, data)
    if master in ready:
        try:
            os.write(1, os.read(master, 65536))
        except OSError:
            break
