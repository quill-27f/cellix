#!/bin/bash
# Screenshots a frame in the middle of a smooth scroll, and the settled page, in Ghostty.
# Usage: harness/guard.sh --gpu -- harness/live/scroll.sh <output-dir> [file]
set -u
out=$1; root=$(cd "$(dirname "$0")/../.." && pwd)
file=${2:-$root/crates/cellix-render/src/grid.rs}
ghostty=/Applications/Ghostty.app/Contents/MacOS/ghostty
mkdir -p "$out"; rm -f "$out/frozen"
cat > "$out/winid.swift" <<'SW'
import CoreGraphics
let pid = Int(CommandLine.arguments[1])!
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as! [[String: Any]]
for w in list where (w[kCGWindowOwnerPID as String] as? Int) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    print(w[kCGWindowNumber as String] as! Int); break
}
SW
"$ghostty" --title=cellix-scroll --window-width=110 --window-height=32 \
  --quit-after-last-window-closed=true \
  -e python3 -I "$root/harness/live/scroll_proxy.py" 3 "$out/frozen" "$root/target/debug/cellix" "$file" >/dev/null 2>&1 &
pid=$!
for _ in $(seq 1 60); do [ -f "$out/frozen" ] && break; python3 -I -c 'import time; time.sleep(.25)'; done
[ -f "$out/frozen" ] && echo "froze mid-scroll" || echo "never froze"
id=$(swift "$out/winid.swift" "$pid" 2>/dev/null)
shot() { if [ -n "$id" ]; then screencapture -x -l"$id" "$1"; else screencapture -x "$1"; fi; }
shot "$out/mid.png" && echo "captured mid (window: ${id:-whole screen})"
python3 -I -c 'import time; time.sleep(9)'
shot "$out/settled.png" && echo captured settled
kill -9 "$pid" 2>/dev/null; wait "$pid" 2>/dev/null
