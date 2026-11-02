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
  } else if (kind === 'attention') {
    await openPane($)
  }
}

/** Runs Cellix for the life of the session. Without it, Claude Code just carries on. */
async function bridge($: Engine) {
  let buffer = ''
  for await (const piece of $.process.spawn({ argv: ['cellix', 'pane'] })) {
    if (!('stream' in piece) || piece.stream !== 'stdout') continue
    buffer += piece.text
    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''
    for (const text of lines) await line($, text)
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
    $.clock.every(REFRESH_MS, () => showCount($))
    const started = await next(e)
    void bridge($).catch(() => undefined)
    return started
  })

  on('command.run', { command: 'cellix' }, async $ => {
    await openPane($)
    return { text: 'Cellix is open. Click it to type there, and press Esc to come back.' }
  })

  on('command.run', { command: 'cellix-comments' }, async $ => {
    const comments = await pending($)
    if (comments === undefined) return { text: 'Cellix is not running in this folder.' }
    if (comments.length === 0) return { text: 'No comments are waiting.' }
    return { text: comments.map(c => `${c.file}:${c.line}: ${c.text}`).join('\n') }
  })

  on('command.run', { command: 'cellix-open' }, async ($, e) => {
    await openPane($)
    const path = `${await $.session.cwd()}/${e.args}`
    send($, [{ type: 'open', path }])
    return { text: `Opened ${e.args} in Cellix.` }
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
    if (source === undefined) return <Text dimColor>Starting Cellix…</Text>
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
