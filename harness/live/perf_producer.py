#!/usr/bin/env python3
"""Prints a numbered line in place, so many times a second, once a gate file exists: the
stand-in for a busy program in `harness/x perf`. Every second it notes how many it has
printed beside the gate, so the run can tell a writer that was held up from one that ran.

    perf_producer.py <rate> <gate> <label>"""
import os, sys, time

rate, gate, label = float(sys.argv[1]), sys.argv[2], sys.argv[3]
while not os.path.exists(gate):
    time.sleep(0.01)
period = 1 / rate
count, due, noted = 0, time.monotonic(), time.monotonic()
while True:
    count += 1
    sys.stdout.write(f"\rbench-output-{count:08d}-{label}")
    sys.stdout.flush()
    now = time.monotonic()
    if now - noted >= 1:
        noted = now
        with open(f"{gate}.{label}", "w") as f:
            f.write(str(count))
    due += period
    if due < now - period:
        due = now + period
    if due > now:
        time.sleep(due - now)
