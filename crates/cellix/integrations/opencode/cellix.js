// Tells a running Cellix what OpenCode is doing: working, waiting for you, or done, and which
// session it is, so Cellix can resume it. Written by `cellix setup opencode`, and taken away by
// `cellix setup opencode --remove`. It never waits for Cellix, and where Cellix isn't
// installed or running it does nothing at all.
import { spawn } from "node:child_process"

const CELLIX = process.env.CELLIX_BIN || "cellix"

/** Hands one piece of news to `cellix hook`, and goes on without waiting for it. */
function tell(kind, fields) {
  try {
    const child = spawn(CELLIX, ["hook", "--agent", "OpenCode", kind], {
      stdio: ["pipe", "ignore", "ignore"],
    })
    child.on("error", () => {})
    child.stdin.on("error", () => {})
    child.stdin.end(JSON.stringify(fields))
  } catch {}
}

/** What a permission or a question asks, in its own words where it has some. */
function asking(p) {
  if (typeof p.title === "string" && p.title) return p.title
  const patterns = Array.isArray(p.patterns) ? p.patterns.join(" ") : ""
  if (typeof p.permission === "string" && p.permission) {
    const verb = { bash: "run", webfetch: "fetch" }[p.permission] ?? p.permission
    return `OpenCode asks to ${verb}${patterns ? ` ${patterns}` : ""}`
  }
  const question = Array.isArray(p.questions) ? p.questions[0]?.question : p.question
  return typeof question === "string" ? question : ""
}

export const Cellix = async ({ directory }) => {
  const commands = new Map()
  const at = (session, more = {}) => ({ session_id: session || "", cwd: directory, ...more })
  return {
    event: async ({ event }) => {
      const p = event?.properties ?? {}
      const session = p.sessionID ?? p.info?.id ?? ""
      switch (event?.type) {
        case "session.created":
          tell("session-start", at(session))
          break
        case "session.status":
          if (p.status?.type === "busy") tell("working", at(session))
          break
        case "permission.asked":
        case "permission.updated":
          tell("notify", at(session, { notification_type: "permission_prompt", message: asking(p) }))
          break
        case "question.asked":
          tell("notify", at(session, { notification_type: "elicitation_dialog", message: asking(p) }))
          break
        case "permission.replied":
        case "question.replied":
          tell("working", at(session))
          break
        case "session.idle":
          tell("stop", at(session))
          break
        case "session.error":
          tell("stop-failed", at(session, { error: p.error?.name ?? "" }))
          break
        case "session.deleted":
          tell("session-end", at(session))
          break
      }
    },
    "chat.message": async (input, output) => {
      const parts = Array.isArray(output?.parts) ? output.parts : []
      const prompt = parts.filter((part) => part?.type === "text").map((part) => part.text).join("\n")
      tell("prompt", at(input?.sessionID, { prompt }))
    },
    "tool.execute.before": async (input, output) => {
      if (input?.callID) commands.set(input.callID, output?.args?.command)
    },
    "tool.execute.after": async (input, output) => {
      const command = commands.get(input?.callID)
      commands.delete(input?.callID)
      const exit = output?.metadata?.exit
      tell("tool-done", at(input?.sessionID, {
        tool_input: { command: typeof command === "string" ? command : "" },
        tool_response: { output: output?.output ?? "", ...(Number.isInteger(exit) ? { exit_code: exit } : {}) },
      }))
    },
  }
}
