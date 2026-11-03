import type { Engine, ImageSource, Register } from 'claude-code'

const PANE = 'cellix'
const REFRESH_MS = 5000
const MAX_IMAGE_CELLS = 255
const INLINE_ROWS = 28
/** More than Claude Code allows, so the dock opens as wide as it can: 45% of the window. */
const DOCK_COLUMNS = 400
const START_TRIES = 40
const START_WAIT_MS = 100

type Comment = { file: string; line: number; text: string }

let source: ImageSource | undefined
/** Why Cellix stopped, when it stopped before it showed anything. */
let stopped: string | undefined
let running = false
let sent = { columns: 0, rows: 0 }
/** Input goes one at a time, so keys can't overtake each other. */
let sending: Promise<unknown> = Promise.resolve()

function send($: Engine, inputs: unknown[]) {
  sending = sending.then(async () => {
    for (const input of inputs) {
      // Cellix may still be starting, as when the session opens with a command.
      for (let tries = 0; tries < START_TRIES; tries++) {
        const { exitCode } = await $.process.run(['cellix', 'pane-send', JSON.stringify(input)])
        if (exitCode === 0) break
        await $.clock.sleep(START_WAIT_MS)
      }
    }
  }).catch(() => undefined)
}

/** Brings the pane up, tall enough to read when it sits above the prompt. */
async function openPane($: Engine) {
  await $.ui.open({ id: PANE, title: 'Cellix', focus: true, rows: INLINE_ROWS, columns: DOCK_COLUMNS })
}

/** The review comments waiting in Cellix, or nothing if no Cellix serves this folder. */
async function pending($: Engine): Promise<Comment[] | undefined> {
  const { exitCode, stdout } = await $.process.run(['cellix', 'comments'])
  return exitCode === 0 ? (JSON.parse(stdout) as Comment[]) : undefined
}

async function showCount($: Engine) {
  const count = (await pending($))?.length ?? 0
  const noun = count === 1 ? 'comment' : 'comments'
  $.ui.status(count > 0 ? `${count} ${noun} waiting` : undefined)
}

async function show($: Engine, next: ImageSource) {
  const first = source === undefined
  source = next
  if (first) {
    $.ui.invalidate('ui.render')
    return
  }
  await $.ui.blit({ requestId: PANE, key: 'view', source: next }).catch(() => undefined)
}

async function line($: Engine, text: string) {
  const [kind, ...rest] = text.split(' ')
  if (kind === 'frame') {
    const [file, width, height, generation] = rest
    await show($, {
      file,
      format: 'rgb',
      width: Number(width),
      height: Number(height),
      generation: Number(generation),
    })
  } else if (kind === 'png') {
    await show($, { png: rest[0] })
  } else if (kind === 'prompt') {
    // Cellix asking the agent something, as Check my review does: a turn of its own.
    const prompt = JSON.parse(rest.join(' ')) as string
    $.clock.after(0, () => {
      void $.prompt.submit({ text: prompt }).catch(() => undefined)
    })
  } else if (kind === 'attention' && rest[0] === 'send') {
    await sendComments($)
  } else if (kind === 'attention') {
    await openPane($)
  }
}

/** Starts a turn for the comments: the prompt hook adds them, as it does to any prompt. */
async function sendComments($: Engine): Promise<number> {
  const count = (await pending($))?.length ?? 0
  // A command holds its own turn until it returns, so the turn starts just after.
  if (count > 0) {
    $.clock.after(0, () => {
      void $.prompt.submit({ text: 'Address the review comments I left in Cellix.' }).catch(() => undefined)
    })
  }
  return count
}

/** A path as typed after a command: the folder's, unless it's already absolute. */
async function resolve($: Engine, typed: string): Promise<string> {
  return typed.startsWith('/') ? typed : `${await $.session.cwd()}/${typed}`
}

/** Runs Cellix for the life of the session. Without it, Claude Code just carries on. */
async function bridge($: Engine) {
  if (running) return
  running = true
  stopped = undefined
  let buffer = ''
  let errors = ''
  try {
    for await (const piece of $.process.spawn({ argv: ['cellix', 'pane'] })) {
      if (!('stream' in piece)) continue
      if (piece.stream === 'stderr') errors += piece.text
      if (piece.stream !== 'stdout') continue
      buffer += piece.text
      const lines = buffer.split('\n')
      buffer = lines.pop() ?? ''
      for (const text of lines) await line($, text)
    }
  } finally {
    running = false
  }
  // Ended before it drew: the pane says why, where it would otherwise wait for good.
  if (source === undefined) {
    stopped = errors.includes('already running')
      ? 'Cellix is already open for this folder in another session. Close it there, then run /cellix again.'
      : `Cellix stopped: ${errors.trim() || 'it exited before it started'}. Run /cellix to try again.`
    $.ui.invalidate('ui.render')
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({ name: 'cellix', description: 'Open Cellix beside the conversation' })
    await $.command.register({
      name: 'cellix-comments',
      description: 'List the review comments waiting in Cellix',
    })
    await $.command.register({ name: 'cellix-open', description: 'Open a file in Cellix' })
    await $.command.register({
      name: 'cellix-send',
      description: 'Send the review comments waiting in Cellix to Claude now',
    })
    $.clock.every(REFRESH_MS, () => showCount($))
    const started = await next(e)
    void bridge($).catch(() => undefined)
    return started
  })

  on('command.run', { command: 'cellix' }, async ($, e) => {
    if (!running && source === undefined) void bridge($).catch(() => undefined)
    await openPane($)
    const typed = e.args.trim()
    if (typed !== '') {
      send($, [{ type: 'open', path: await resolve($, typed) }])
      return { text: `Opened ${typed} in Cellix. Click it to type there, and press Esc to come back.` }
    }
    return { text: 'Cellix is open. Click it to type there, and press Esc to come back.' }
  })

  on('command.run', { command: 'cellix-send' }, async $ => {
    if ((await pending($)) === undefined) return { text: 'Cellix is not running in this folder.' }
    const count = await sendComments($)
    if (count === 0) return { text: 'No comments are waiting.' }
    return { text: `Sending ${count} ${count === 1 ? 'comment' : 'comments'} to Claude.` }
  })

  on('command.run', { command: 'cellix-comments' }, async $ => {
    const comments = await pending($)
    if (comments === undefined) return { text: 'Cellix is not running in this folder.' }
    if (comments.length === 0) return { text: 'No comments are waiting.' }
    return { text: comments.map(c => `${c.file}:${c.line}: ${c.text}`).join('\n') }
  })

  on('command.run', { command: 'cellix-open' }, async ($, e) => {
    const typed = e.args.trim()
    if (typed === '') return { text: 'Name a file: /cellix-open docs/plan.md' }
    await openPane($)
    send($, [{ type: 'open', path: await resolve($, typed) }])
    return { text: `Opened ${typed} in Cellix.` }
  })

  on('ui.message', async ($, e, next) => {
    if (e.requestId !== PANE) return next(e)
    send($, e.data as unknown[])
    return {}
  })

  on('ui.scroll', { requestId: PANE }, ($, e) => {
    send($, [{ type: 'scroll', by: e.by ?? 0 }])
    return {}
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Client, Image, Text } = $.ui.resolve(e)
    const columns = Math.min(e.props.bodyColumns, MAX_IMAGE_CELLS)
    const rows = Math.min(e.props.scroll.bodyRows, MAX_IMAGE_CELLS)
    if (columns !== sent.columns || rows !== sent.rows) {
      sent = { columns, rows }
      send($, [{ type: 'resize', columns, rows }])
    }
    if (source === undefined) return <Text dimColor>{stopped ?? 'Starting Cellix…'}</Text>
    return (
      <Box position="relative" width={columns} height={rows}>
        <Image key="view" source={source} columns={columns} rows={rows} alt="Cellix" />
        <Box position="absolute" top={0} left={0}>
          <Client key="input" module="./pane-input.tsx" width={columns} height={rows} />
        </Box>
      </Box>
    )
  })
}
