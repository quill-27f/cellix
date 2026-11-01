#!/bin/bash
# Screenshots the real binary in Ghostty, tmux and Zellij. Opens windows briefly.
# Usage: harness/guard.sh --gpu -- harness/visual.sh <output-dir> [case]
set -u
out=$1; only=${2:-}
root=$(cd "$(dirname "$0")/.." && pwd)
bin=$root/target/debug/cellix
ghostty=/Applications/Ghostty.app/Contents/MacOS/ghostty
tmux=/opt/homebrew/bin/tmux; zellij=/opt/homebrew/bin/zellij
target=$root/crates/cellix-render/src/grid.rs
mkdir -p "$out"
printf 'set -g allow-passthrough on\nset -g status off\n' > "$out/tmux.conf"
printf 'show_startup_tips false\nshow_release_notes false\npane_frames false\n' > "$out/zellij.kdl"
printf 'layout {\n pane command="%s" {\n args "%s"\n }\n}\n' "$bin" "$target" > "$out/layout.kdl"

cat > "$out/winid.swift" <<'SW'
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    print(w[kCGWindowNumber as String] as! Int); break
}
SW

run() { # name, command...
  name=$1; shift
  if [ -n "$only" ] && [ "$only" != "$name" ]; then return; fi
  "$ghostty" --title="cellix-$name" --window-width=110 --window-height=32 \
    --quit-after-last-window-closed=true -e "$@" >/dev/null 2>&1 &
  pid=$!
  python3 -I -c 'import time; time.sleep(8)'
  id=$(swift "$out/winid.swift" "$pid" 2>/dev/null)
  if [ -n "$id" ]; then
    screencapture -x -l"$id" "$out/$name.png" && echo "$name: captured" || echo "$name: capture failed"
  else
    echo "$name: no window found"
  fi
  kill -9 "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

run ghostty env PATH="/opt/homebrew/bin:$PATH" "$bin" "$target"
run cells env CELLIX_TIER=cells "$bin" "$target"
run tmux env PATH="/opt/homebrew/bin:$PATH" "$tmux" -L cellix-visual -f "$out/tmux.conf" new-session "$bin $target"
"$tmux" -L cellix-visual kill-server 2>/dev/null
run zellij "$zellij" --config "$out/zellij.kdl" --session cellix-visual --new-session-with-layout "$out/layout.kdl"
"$zellij" delete-session --force cellix-visual 2>/dev/null
