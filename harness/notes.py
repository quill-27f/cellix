#!/usr/bin/env python3
"""Prints a release's notes: its section of CHANGELOG.md, then how to install it. Exits 1 when
the changelog has no section for the version, which CI checks before anything is released.

    notes.py <version> [CHANGELOG.md]
"""
import re, sys

version = sys.argv[1]
path = sys.argv[2] if len(sys.argv) > 2 else "CHANGELOG.md"
with open(path) as f:
    text = f.read()
found = re.search(r"(?ms)^## \[%s\][^\n]*\n(.*?)(?=^## \[|^\[[^\]]+\]: |\Z)" % re.escape(version), text)
if not found or not found.group(1).strip():
    sys.exit(f"{path} has no section for {version}")

DOWNLOAD = "https://github.com/quill-27f/cellix/releases/download/v{}/cellix-{}.{}"
targets = [
    ("macOS, Apple silicon", "aarch64-apple-darwin", "tar.gz"),
    ("macOS, Intel", "x86_64-apple-darwin", "tar.gz"),
    ("Linux, ARM (static)", "aarch64-unknown-linux-musl", "tar.gz"),
    ("Linux, x86-64 (static)", "x86_64-unknown-linux-musl", "tar.gz"),
    # Paused while no one tests on Windows (2026-11-09), with its build in release.yml.
    # ("Windows, x86-64", "x86_64-pc-windows-msvc", "zip"),
]
print(found.group(1).strip())
print(f"""
## Install

```sh
brew install quill-27f/tap/cellix    # brew upgrade cellix if you have it
cargo install --locked --git https://github.com/quill-27f/cellix --tag v{version} cellix
```

Or take the binary for your machine, which needs nothing else installed:

| Machine | Download |
|---|---|""")
for name, target, kind in targets:
    print(f"| {name} | [cellix-{target}.{kind}]({DOWNLOAD.format(version, target, kind)}) |")
print(f"""
```sh
mkdir -p ~/.local/bin
curl -fL {DOWNLOAD.format(version, "<target>", "tar.gz")} | tar xz -C ~/.local/bin ./cellix
```

There's no Windows build of this release; on Windows, `cargo install` as above.

Then `cellix setup claude`, `codex` or `gemini` adds it to your agent. `checksums.txt` has the
SHA-256 of each archive.""")
