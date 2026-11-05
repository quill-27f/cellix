import type { Engine, ImageSource, Register } from 'claude-code'

const PANE = 'cellix'
const REFRESH_MS = 5000
const MAX_IMAGE_CELLS = 255
const INLINE_ROWS = 28
/** More than Claude Code allows, so the dock opens as wide as it can: 45% of the window. */
const DOCK_COLUMNS = 400
/** How long after the pane is first drawn to check that its pictures show. */
const CHECK_MS = 300
const START_TRIES = 40
const START_WAIT_MS = 100
/** What this mod and `cellix pane` say to each other; Cellix's `pane::PROTOCOL`. */
const PROTOCOL = '2'

type Comment = { file: string; line: number; text: string }
/** A frame as cells, for where pictures can't show: drawn as text. */
type Cells = { columns: number; rows: number; cells: string }
/** A frame as a picture a row, which Cellix sends over SSH and in tmux. Each row is named by
 * what it shows, and a picture (PNG, base64) comes only with a name not held yet. Drawn under
 * those names, a row that moves keeps its picture: Claude Code uploads each name once. */
type Rows = { columns: number; rows: number; names: string[]; pictures: Map<string, string> }

let source: ImageSource | undefined
let cells: Cells | undefined
let rows: Rows | undefined
/** Whether a drawn row has shown that pictures show here. */
let checked = false
/** Why Cellix stopped, when it stopped before it showed anything. */
let stopped: string | undefined
let running = false
let sent = { columns: 0, rows: 0 }
/** Input waiting to go. It goes in order, all that waits in one send, so a click never
 * queues behind a send for each move before it. */
let waiting: unknown[] = []
let sending = false
/** How Cellix is started: by name, or where setup found it, when Claude Code's PATH lacks it. */
let program = 'cellix'

function send($: Engine, inputs: unknown[]) {
  waiting.push(...inputs)
  if (sending) return
  sending = true
  void (async () => {
    try {
      while (waiting.length > 0) {
        const batch = waiting
        waiting = []
        // Cellix may still be starting, as when the session opens with a command, and may
        // yet be found somewhere other than the PATH.
        for (let tries = 0; tries < START_TRIES; tries++) {
          const done = await $.process.run([program, 'pane-send', JSON.stringify(batch)]).catch(() => undefined)
          if (done?.exitCode === 0) break
          await $.clock.sleep(START_WAIT_MS)
        }
      }
    } catch {
      waiting = []
    } finally {
      sending = false
    }
  })()
}

/** Whether Cellix has drawn anything yet. */
function shown(): boolean {
  return source !== undefined || cells !== undefined || rows !== undefined
}

/** Brings the pane up, tall enough to read when it sits above the prompt. */
async function openPane($: Engine) {
  await $.ui.open({ id: PANE, title: 'Cellix', focus: true, rows: INLINE_ROWS, columns: DOCK_COLUMNS })
}

/** The review comments waiting in Cellix, or nothing if no Cellix serves this folder. */
async function pending($: Engine): Promise<Comment[] | undefined> {
  const done = await $.process.run([program, 'comments']).catch(() => undefined)
  return done?.exitCode === 0 ? (JSON.parse(done.stdout) as Comment[]) : undefined
}

async function showCount($: Engine) {
  const count = (await pending($))?.length ?? 0
  const noun = count === 1 ? 'comment' : 'comments'
  $.ui.status(count > 0 ? `${count} ${noun} waiting` : undefined)
}

async function show($: Engine, next: ImageSource) {
  const first = source === undefined
  source = next
  cells = undefined
  rows = undefined
  if (first) {
    $.ui.invalidate('ui.render')
    return
  }
  await $.ui.blit({ requestId: PANE, key: 'view', source: next }).catch(() => undefined)
}

/** Repaints the cells in place, or draws the pane again when their size changed. */
async function showCells($: Engine, next: Cells) {
  const same = cells !== undefined && cells.columns === next.columns && cells.rows === next.rows
  cells = next
  source = undefined
  rows = undefined
  if (same) {
    const { deny } = await $.ui
      .blit({ requestId: PANE, key: 'view', cells: next.cells, columns: next.columns, rows: next.rows })
      .catch(() => ({ deny: 'failed' }))
    if (deny === undefined) return
  }
  $.ui.invalidate('ui.render')
}

/** Takes a frame of named rows: new pictures join those held, and what left the screen goes,
 * as Cellix expects. The pane is drawn again, and Claude Code sends the terminal only the
 * pictures it hasn't shown. */
function showRows($: Engine, columns: number, height: number, list: string, parts: string[]) {
  const names = list.split(',')
  const pictures = rows !== undefined && rows.columns === columns ? rows.pictures : new Map<string, string>()
  for (const part of parts) {
    const at = part.indexOf(':')
    pictures.set(part.slice(0, at), part.slice(at + 1))
  }
  const onScreen = new Set(names)
  for (const name of [...pictures.keys()]) if (!onScreen.has(name)) pictures.delete(name)
  rows = { columns, rows: height, names, pictures }
  source = undefined
  cells = undefined
  $.ui.invalidate('ui.render')
  if (!checked) {
    checked = true
    $.clock.after(CHECK_MS, () => void check($))
  }
}

/** Swaps a drawn row for itself, which sends the terminal nothing, to learn whether pictures
 * show here. Where they can't, Cellix is asked for cells and told why. */
async function check($: Engine) {
  const frame = rows
  const name = frame?.names[0]
  const png = name === undefined ? undefined : frame?.pictures.get(name)
  if (png === undefined) return
  const { deny } = await $.ui
    .blit({ requestId: PANE, key: `${name}.0`, source: { png } })
    .catch(() => ({ deny: undefined }))
  if (deny?.includes('draws its alt')) {
    send($, [{ type: 'tier', tier: 'cells', why: deny.replace(/^.*draws its alt here: /, '') }])
  } else if (deny !== undefined) {
    // Not drawn yet, as when the pane isn't open: the next frame asks again.
    checked = false
  }
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
  } else if (kind === 'rows') {
    const [columns, height, list, ...parts] = rest
    showRows($, Number(columns), Number(height), list, parts)
  } else if (kind === 'cells') {
    const [columns, rows, packed] = rest
    await showCells($, { columns: Number(columns), rows: Number(rows), cells: packed })
  } else if (kind === 'fill') {
    // Ask Agent: what the selection is about, in the prompt box for the person to finish.
    const text = JSON.parse(rest.join(' ')) as string
    await $.prompt.fill({ text, mode: 'insert' }).catch(() => undefined)
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

/** The ways to start Cellix, in order: by name, then where setup found it. */
async function programs($: Engine): Promise<string[]> {
  const found = await $.env.get('CELLIX_BIN').catch(() => undefined)
  return found && found !== 'cellix' ? ['cellix', found] : ['cellix']
}

/** Runs Cellix for the life of the session. Without it, Claude Code just carries on. */
async function bridge($: Engine) {
  if (running) return
  running = true
  stopped = undefined
  let buffer = ''
  let errors = ''
  /** Why Cellix couldn't be started at all, as when it isn't on Claude Code's PATH. */
  let unstarted: string | undefined
  try {
    for (const argv0 of await programs($)) {
      let started = false
      try {
        const env = { CELLIX_MOD: PROTOCOL }
        for await (const piece of $.process.spawn({ argv: [argv0, 'pane'], env })) {
          if (!started) {
            started = true
            program = argv0
            unstarted = undefined
          }
          if (!('stream' in piece)) continue
          if (piece.stream === 'stderr') errors += piece.text
          if (piece.stream !== 'stdout') continue
          buffer += piece.text
          const lines = buffer.split('\n')
          buffer = lines.pop() ?? ''
          for (const text of lines) await line($, text)
        }
        break
      } catch (error) {
        if (started) throw error
        unstarted = error instanceof Error ? error.message : String(error)
      }
    }
  } catch (error) {
    errors ||= error instanceof Error ? error.message : String(error)
  } finally {
    running = false
  }
  // Ended before it drew: the pane says why, where it would otherwise wait for good.
  if (!shown()) {
    stopped = errors.includes('already running')
      ? 'Cellix is already open for this folder in another session. Close it there, then run /cellix again.'
      : unstarted !== undefined
        ? `${/ENOENT|not found/i.test(unstarted) ? "Claude Code can't find cellix on the PATH it was started with" : `Claude Code couldn't start cellix: ${unstarted}`}. Run \`cellix setup claude\` in a shell where \`cellix\` works, then restart Claude Code.`
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
    if (!running && !shown()) void bridge($).catch(() => undefined)
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
    const { Box, Client, Image, Raster, Text } = $.ui.resolve(e)
    const width = Math.min(e.props.bodyColumns, MAX_IMAGE_CELLS)
    const height = Math.min(e.props.scroll.bodyRows, MAX_IMAGE_CELLS)
    if (width !== sent.columns || height !== sent.rows) {
      sent = { columns: width, rows: height }
      send($, [{ type: 'resize', columns: width, rows: height }])
    }
    if (!shown()) return <Text dimColor>{stopped ?? 'Starting Cellix…'}</Text>
    const frame = rows
    // A name on screen twice, as blank lines are, is drawn under one key each.
    const seen = new Map<string, number>()
    const view =
      frame !== undefined ? (
        <Box flexDirection="column">
          {frame.names.map(name => {
            const nth = seen.get(name) ?? 0
            seen.set(name, nth + 1)
            const png = frame.pictures.get(name)
            const key = `${name}.${nth}`
            return png === undefined ? (
              <Box key={key} width={frame.columns} height={1} />
            ) : (
              <Image key={key} source={{ png }} columns={frame.columns} rows={1} alt=" " />
            )
          })}
        </Box>
      ) : cells !== undefined ? (
        <Raster key="view" columns={cells.columns} rows={cells.rows} cells={cells.cells} />
      ) : (
        <Image key="view" source={source!} columns={width} rows={height} alt="Cellix" />
      )
    return (
      <Box position="relative" width={width} height={height}>
        {view}
        <Box position="absolute" top={0} left={0}>
          <Client key="input" module="./pane-input.tsx" width={width} height={height} />
        </Box>
      </Box>
    )
  })
}
