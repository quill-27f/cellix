#!/usr/bin/env python3
"""Sets the version everywhere it's written: the workspace, the crates' dependencies on each
other, Cargo.lock and the three agent plugins. Pushing the result to main releases it.

    bump.py <version>
"""
import json, os, re, sys

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
new = sys.argv[1]
if not re.fullmatch(r"\d+\.\d+\.\d+", new):
    sys.exit(f"not a version: {new}")


def edit(path, change):
    path = os.path.join(root, path)
    with open(path) as f:
        text = f.read()
    changed = change(text)
    if changed == text:
        sys.exit(f"nothing to change in {path}")
    with open(path, "w") as f:
        f.write(changed)


with open(os.path.join(root, "Cargo.toml")) as f:
    old = re.search(r'(?m)^version = "([^"]+)"', f.read()).group(1)
if old == new:
    sys.exit(f"already {new}")

edit("Cargo.toml", lambda t: t.replace(f'version = "{old}"', f'version = "{new}"', 1))
crates = sorted(os.listdir(os.path.join(root, "crates")))
for crate in crates:
    manifest = f"crates/{crate}/Cargo.toml"
    with open(os.path.join(root, manifest)) as f:
        if f'version = "{old}"' not in f.read():
            continue
    # Only the dependencies on the workspace's own crates, which carry a path.
    edit(manifest, lambda t: "\n".join(
        line.replace(f'version = "{old}"', f'version = "{new}"') if "path =" in line else line
        for line in t.split("\n")))
edit("Cargo.lock", lambda t: re.sub(
    r'(name = "(?:%s)"\nversion = )"%s"' % ("|".join(map(re.escape, crates)), re.escape(old)),
    rf'\1"{new}"', t))
for plugin in ["claude-code/.claude-plugin/plugin.json", "codex/plugin.json", "gemini/gemini-extension.json"]:
    edit(f"crates/cellix/integrations/{plugin}",
         lambda t: t.replace(f'"version": "{old}"', f'"version": "{new}"', 1))

changelog = os.path.join(root, "CHANGELOG.md")
if not os.path.exists(changelog) or f"## [{new}]" not in open(changelog).read():
    print(f"CHANGELOG.md needs a '## [{new}] - <date>' section before this is pushed.")
print(f"{old} -> {new}")
