#!/usr/bin/env python3
"""Makes a project with one long document and many comments on it, to time a long list.

Usage: harness/many_comments.py <folder> <how many>
Then: harness/guard.sh -- cargo run --release -p cellix --example wheel <folder> docs/paper.md
"""
import json
import os
import random
import sys

root, count = sys.argv[1], int(sys.argv[2])
random.seed(7)
WORDS = (
    "the attention weights sum to one over each head so a query reads its keys in proportion "
    "and layers add what they read to the stream that later layers read from in turn which "
    "is why a probe at one depth says little about another and the claim needs a control"
).split()


def sentence(low, high):
    words = (random.choice(WORDS) for _ in range(random.randint(low, high)))
    return " ".join(words).capitalize() + "."


os.makedirs(os.path.join(root, "docs"), exist_ok=True)
os.makedirs(os.path.join(root, ".cellix"), exist_ok=True)
lines = ["# A long paper", ""]
for i in range(count * 3):
    lines.append(sentence(12, 30))
    if i % 4 == 3:
        lines.append("")
with open(os.path.join(root, "docs", "paper.md"), "w") as out:
    out.write("\n".join(lines) + "\n")

kinds = ["", "Clarity", "Correctness", "Evidence", "Structure", "Reproducibility"]
texts = [(number, line) for number, line in enumerate(lines, 1) if line and line[0] != "#"]
threads = []
for i in range(count):
    number, line = texts[(i * len(texts)) // count]
    parts = line.split()
    start = random.randint(0, max(0, len(parts) - 6))
    replies = [
        {
            "author": "Claude" if r % 2 == 0 else "You",
            "text": " ".join(sentence(8, 40) for _ in range(random.randint(1, 3))),
        }
        for r in range(random.choice([0, 0, 1, 1, 2, 3]))
    ]
    threads.append(
        {
            "id": i + 1,
            "file": "docs/paper.md",
            "line": number,
            "quote": " ".join(parts[start : start + random.randint(2, 6)]),
            "text": " ".join(sentence(6, 30) for _ in range(random.randint(1, 2)))
            + (" :+1: see https://example.com/a/b" if i % 7 == 0 else ""),
            "replies": replies,
            "kind": random.choice(kinds),
            "resolved": i % 11 == 0,
        }
    )
with open(os.path.join(root, ".cellix", "threads.json"), "w") as out:
    json.dump(threads, out)
print(f"{len(threads)} comments on {len(lines)} lines in {root}")
