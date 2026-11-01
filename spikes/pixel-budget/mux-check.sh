#!/bin/bash
# Renders static tile pages through Ghostty, tmux and Zellij, screenshotting each.
# Usage: mux-check.sh <output-dir> [only-this-case]
set -u
out=$1; only=${2:-}; bin=$PWD/target/release/pixel-budget; ghostty=/Applications/Ghostty.app/Contents/MacOS/ghostty
tmux=/opt/homebrew/bin/tmux; zellij=/opt/homebrew/bin/zellij
mkdir -p "$out"
printf 'set -g allow-passthrough on\nset -g status off\n' > "$out/tmux.conf"
printf 'show_startup_tips false\nshow_release_notes false\npane_frames false\n' > "$out/zellij.kdl"

cat > "$out/winid.swift" <<'SW'
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    print(w[kCGWindowNumber as String] as! Int); break
}
SW

layout() { printf 'layout {\n pane command="%s" {\n args "show" "%s"\n }\n}\n' "$bin" "$1" > "$out/layout-$1.kdl"; }
layout direct; layout placeholder

run() { # name, command...
  name=$1; shift
  if [ -n "$only" ] && [ "$only" != "$name" ]; then return; fi
  "$ghostty" --title="cellix-$name" --window-width=100 --window-height=26 --quit-after-last-window-closed=true -e "$@" >/dev/null 2>&1 &
  pid=$!
  python3 -I -c 'import time; time.sleep(9)'
  id=$(swift "$out/winid.swift" "$pid" 2>/dev/null)
  if [ -n "$id" ]; then screencapture -x -l"$id" "$out/$name.png" && echo "$name: captured" || echo "$name: capture failed"
  else echo "$name: no window found"; fi
  kill -9 "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
}

run ghostty-direct "$bin" show direct
run ghostty-placeholder "$bin" show placeholder
run tmux-direct "$tmux" -L cellix-test -f "$out/tmux.conf" new-session "$bin show direct --wrap"
"$tmux" -L cellix-test kill-server 2>/dev/null
run tmux-placeholder "$tmux" -L cellix-test -f "$out/tmux.conf" new-session "$bin show placeholder --wrap"
"$tmux" -L cellix-test kill-server 2>/dev/null
run zellij-direct "$zellij" --config "$out/zellij.kdl" --session cellix-test-a --new-session-with-layout "$out/layout-direct.kdl"
"$zellij" delete-session --force cellix-test-a 2>/dev/null
run zellij-placeholder "$zellij" --config "$out/zellij.kdl" --session cellix-test-b --new-session-with-layout "$out/layout-placeholder.kdl"
"$zellij" delete-session --force cellix-test-b 2>/dev/null
