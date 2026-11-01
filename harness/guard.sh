#!/usr/bin/env bash
# Runs a command only when the machine can afford it, and stops it if that changes.
#
#   harness/guard.sh [--mem-mb N] [--timeout S] [--wait S] [--gpu] [--gpu-mb N] [--label L] -- cmd...
#
# One guarded job runs at a time across every worktree. macOS has no cgroups and
# ignores `ulimit -v`, so memory is enforced by watching the whole process tree.
#
# Exit codes: the command's own, or
#   75  never admitted: the machine stayed too busy for --wait seconds
#   124 killed: --timeout
#   137 killed: over the memory or GPU budget, or the system ran out of memory
set -u

mem_mb=${CELLIX_GUARD_MEM_MB:-8192}
timeout_s=${CELLIX_GUARD_TIMEOUT:-1800}
wait_s=${CELLIX_GUARD_WAIT:-900}
gpu=0
gpu_mb=1024
label=""

while [ $# -gt 0 ]; do
  case $1 in
    --mem-mb) mem_mb=$2; shift 2 ;;
    --timeout) timeout_s=$2; shift 2 ;;
    --wait) wait_s=$2; shift 2 ;;
    --gpu) gpu=1; shift ;;
    --gpu-mb) gpu_mb=$2; shift 2 ;;
    --label) label=$2; shift 2 ;;
    --) shift; break ;;
    *) echo "guard: unknown option $1" >&2; exit 2 ;;
  esac
done
[ $# -gt 0 ] || { echo "guard: no command given" >&2; exit 2; }

# CI runners are disposable and the probes below are macOS only.
if [ -n "${CI:-}" ] || [ "$(uname)" != Darwin ]; then exec "$@"; fi

# Lives in the common git dir so every worktree shares one lock.
state=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null)/cellix-guard
mkdir -p "$state" || { echo "guard: cannot create $state" >&2; exit 2; }
lock=$state/lock
log=$state/guard.log

free_pct() { memory_pressure -Q 2>/dev/null | awk '/free percentage/ { gsub("%", ""); print $NF }'; }
pressure() { sysctl -n kern.memorystatus_vm_pressure_level 2>/dev/null; } # 1 normal, 2 warn, 4 critical
disk_gb() { df -g . | awk 'NR == 2 { print $4 }'; }
gpu_stat() { ioreg -r -d 1 -w 0 -c IOAccelerator 2>/dev/null | grep -o "\"$1\"=[0-9]*" | head -1 | cut -d= -f2; }

# Prints "total_mb pid pid ..." for pid and all its descendants. Walks parents
# rather than the process group so PTY children in their own session still count.
tree() {
  ps -A -o pid=,ppid=,rss= | awk -v root="$1" '
    { ppid[$1] = $2; rss[$1] = $3 }
    END {
      keep[root] = 1
      do { grew = 0
        for (p in ppid) if (!(p in keep) && (ppid[p] in keep)) { keep[p] = 1; grew = 1 }
      } while (grew)
      for (p in keep) if (p in rss) { total += rss[p]; list = list " " p }
      print int(total / 1024) list
    }'
}

acquire() {
  while ! mkdir "$lock" 2>/dev/null; do
    holder=$(cat "$lock/pid" 2>/dev/null)
    if [ -n "$holder" ] && ! kill -0 "$holder" 2>/dev/null; then rm -rf "$lock"; continue; fi
    [ "$SECONDS" -lt "$wait_s" ] || return 1
    sleep 2
  done
  echo $$ > "$lock/pid"
}

admit() {
  while :; do
    free=$(free_pct); level=$(pressure); disk=$(disk_gb)
    reason=""
    [ "${free:-0}" -ge 20 ] || reason="memory free ${free:-?}%"
    [ "${level:-1}" -lt 4 ] || reason="critical memory pressure"
    [ "${disk:-0}" -ge 20 ] || reason="disk free ${disk:-?} GB"
    if [ "$gpu" = 1 ] && [ "$(gpu_stat 'Device Utilization %')" -ge 70 ] 2>/dev/null; then
      reason="GPU busy"
    fi
    [ -n "$reason" ] || return 0
    [ "$SECONDS" -lt "$wait_s" ] || { echo "guard: not admitted: $reason" >&2; return 1; }
    echo "guard: waiting: $reason" >&2
    sleep 15
  done
}

record() { printf '%s\t%s\t%s\texit=%s\tpeak=%sMB\t%ss\tjobs=%s\t%s\n' \
  "$(date '+%F %T')" "${label:-$1}" "$(pwd -P | sed 's|.*/||')" "$2" "$peak" "$SECONDS" "$jobs" "$3" >> "$log"; }

# Only ever signals real descendant pids. An empty tree must stay empty: `kill 0`
# would signal the guard's own process group, including whatever launched it.
signal_tree() {
  for p in $(tree "$child" | cut -s -d' ' -f2-); do
    [ "$p" -gt 1 ] && [ "$p" != $$ ] && kill "$1" "$p" 2>/dev/null
  done
}

stop_tree() {
  signal_tree -TERM
  sleep 2
  signal_tree -KILL
}

peak=0
jobs=0
acquire || { echo "guard: another guarded job held the lock for ${wait_s}s" >&2; exit 75; }
trap 'rm -rf "$lock"' EXIT
trap '[ -n "${child:-}" ] && stop_tree; exit 130' INT TERM
admit || { record "$1" 75 "not admitted"; exit 75; }

# Fewer parallel rustc processes when memory is tighter; each can peak near 1 GB.
if [ "$free" -ge 40 ]; then jobs=4; else jobs=2; fi
gpu_base=$(gpu_stat 'In use system memory'); recov_base=$(gpu_stat recoveryCount)
SECONDS=0

set -m # the child gets its own process group, so ^C reaches the guard first
CARGO_BUILD_JOBS=$jobs RUST_TEST_THREADS=$jobs CELLIX_GPU=$gpu \
  nice -n 10 taskpolicy -c utility "$@" < /dev/null &
child=$!
set +m

killed=""
tick=0
while kill -0 "$child" 2>/dev/null; do
  sleep 0.5
  tick=$((tick + 1))
  sample=$(tree "$child"); mb=${sample%% *}
  [ "${mb:-0}" -le "$peak" ] || peak=$mb
  if [ "${mb:-0}" -gt "$mem_mb" ]; then killed="over memory budget: ${mb} MB > ${mem_mb} MB"
  elif [ "$SECONDS" -ge "$timeout_s" ]; then killed="timeout after ${timeout_s}s"
  elif [ $((tick % 6)) = 0 ]; then
    free=$(free_pct); level=$(pressure)
    if [ "${level:-1}" -ge 4 ] || [ "${free:-100}" -lt 10 ]; then
      killed="system low on memory: free ${free:-?}%, pressure ${level:-?}"
    elif [ "$gpu" = 1 ]; then
      now=$(gpu_stat 'In use system memory'); recov=$(gpu_stat recoveryCount)
      if [ "${recov:-0}" -gt "${recov_base:-0}" ]; then killed="GPU reset detected"
      elif [ $(( (${now:-0} - ${gpu_base:-0}) / 1048576 )) -gt "$gpu_mb" ]; then
        killed="over GPU budget: ${gpu_mb} MB"
      fi
    fi
  fi
  if [ -n "$killed" ]; then
    echo "guard: stopping job: $killed" >&2
    stop_tree
    wait "$child" 2>/dev/null
    code=137; case $killed in timeout*) code=124 ;; esac
    record "$1" "$code" "$killed"
    exit "$code"
  fi
done
wait "$child"
code=$?
child=""
record "$1" "$code" ok
exit "$code"
