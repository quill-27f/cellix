#!/usr/bin/env bash
# PreToolUse hook: refuses compiles and test runs that skip the resource guard.
cmd=$(jq -r '.tool_input.command // ""')
heavy='(^|[;&|(]|&&)[[:space:]]*(env[[:space:]]+[^;&|]*)?((cargo[[:space:]]+(\+[^[:space:]]+[[:space:]]+)?(build|b|check|c|test|t|clippy|run|r|bench|nextest|install|doc)([[:space:]]|$))|(rustc[[:space:]]))'
if printf '%s' "$cmd" | grep -Eq "$heavy" && ! printf '%s' "$cmd" | grep -Eq 'harness/(x|guard\.sh)'; then
  jq -n '{hookSpecificOutput: {hookEventName: "PreToolUse", permissionDecision: "deny",
    permissionDecisionReason: "Compile and test through the resource guard: harness/x check|test|build, or harness/guard.sh -- <command>. See AGENTS.md."}}'
fi
exit 0
