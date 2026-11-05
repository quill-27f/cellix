import { expect, mock, test } from 'claude-code/testing'

const WAITING = [{ file: 'plan.md', line: 3, text: 'Split this step' }]

test('sending comments starts a turn, which the prompt hook fills', async ($, on) => {
  const submitted: string[] = []
  const clock = mock.clock(on)
  on('process.run', async () => ({ value: { exitCode: 0, stdout: JSON.stringify(WAITING), stderr: '' } }))
  on('prompt.submit', async (_, e) => {
    submitted.push(e.text)
    return { text: e.text }
  })
  const result = await $.command.run({ command: 'cellix-send', args: '' })
  expect(result.text).toBe('Sending 1 comment to Claude.')
  await clock.advance(1)
  expect(submitted).toEqual(['Address the review comments I left in Cellix.'])
})

test('with nothing waiting, sending says so and starts no turn', async ($, on) => {
  mock.clock(on)
  on('process.run', async () => ({ value: { exitCode: 0, stdout: '[]', stderr: '' } }))
  on('prompt.submit', async () => {
    throw new Error('no turn should start')
  })
  const result = await $.command.run({ command: 'cellix-send', args: '' })
  expect(result.text).toBe('No comments are waiting.')
})

test('a second session in the same folder says why, rather than waiting', async ($, on) => {
  on('process.spawn', async function* () {
    yield { stream: 'stderr' as const, text: 'cellix: already running\n' }
    return { value: { code: 1, signal: null } }
  })
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('process.run', async () => ({ value: { exitCode: 1, stdout: '', stderr: '' } }))
  await $.command.run({ command: 'cellix', args: '' })
  const pane = await $.ui.mount({
    plugin: 'cellix',
    surface: 'terminal',
    component: 'Pane',
    requestId: 'cellix',
    props: { bodyColumns: 80, scroll: { bodyRows: 20 } } as never,
  })
  const shown = await pane.find({ type: 'Text', text: /already open for this folder/ })
  expect(shown).toBeDefined()
})

test('/cellix with a file opens it in the pane', async ($, on) => {
  const sent: string[] = []
  const clock = mock.clock(on)
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('session.cwd', async () => ({ value: '/work' }))
  on('process.spawn', async function* () {
    return { value: { code: 0, signal: null } }
  })
  on('process.run', async (_, e) => {
    sent.push(e.argv.join(' '))
    return { value: { exitCode: 0, stdout: '', stderr: '' } }
  })
  const result = await $.command.run({ command: 'cellix', args: 'docs/plan.md' })
  expect(result.text).toContain('Opened docs/plan.md')
  await clock.settle()
  expect(sent).toContain('cellix pane-send [{"type":"open","path":"/work/docs/plan.md"}]')
})

/** A frame of cells as Cellix sends it over SSH: one cell, a character and two colours. */
function cellsLine(ch: string, columns = 1, rows = 1): string {
  const words = new Uint32Array(columns * rows * 3)
  for (let i = 0; i < columns * rows; i++) words.set([ch.codePointAt(0)!, 0xffffff, 0x000000], i * 3)
  return `cells ${columns} ${rows} ${new Uint8Array(words.buffer).toBase64()}\n`
}

test('over SSH the pane is drawn as cells, and the next frame repaints them in place', async ($, on) => {
  const blits: unknown[] = []
  const clock = mock.clock(on)
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('process.run', async () => ({ value: { exitCode: 0, stdout: '[]', stderr: '' } }))
  on('ui.blit', async (_, e) => {
    blits.push(e)
    return { value: {} }
  })
  on('process.spawn', async function* () {
    yield { stream: 'stdout' as const, text: cellsLine('a', 2, 1) }
    await clock.sleep(10)
    yield { stream: 'stdout' as const, text: cellsLine('b', 2, 1) }
    return { value: { code: 0, signal: null } }
  })
  await $.command.run({ command: 'cellix', args: '' })
  await clock.advance(1)
  const pane = await $.ui.mount({
    plugin: 'cellix',
    surface: 'terminal',
    component: 'Pane',
    requestId: 'cellix',
    props: { bodyColumns: 80, scroll: { bodyRows: 20 } } as never,
  })
  expect(await pane.find({ type: 'Raster' })).toBeDefined()
  await clock.advance(20)
  expect(blits.length).toBe(1)
})

test("Ask Agent's words go in the prompt box, unsent", async ($, on) => {
  const filled: string[] = []
  const clock = mock.clock(on)
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('process.run', async () => ({ value: { exitCode: 0, stdout: '[]', stderr: '' } }))
  on('prompt.fill', async (_, e) => {
    filled.push(e.text)
    return { isFilled: true, text: e.text, cursor: e.text.length }
  })
  on('prompt.submit', async () => {
    throw new Error('nothing is sent')
  })
  on('process.spawn', async function* () {
    yield { stream: 'stdout' as const, text: `fill ${JSON.stringify('@plan.md line 3, "cache": ')}\n` }
    return { value: { code: 0, signal: null } }
  })
  await $.command.run({ command: 'cellix', args: '' })
  await clock.settle()
  expect(filled).toEqual(['@plan.md line 3, "cache": '])
})

test('input that comes in while a send runs goes in the next send, all together', async ($, on) => {
  const sent: string[] = []
  const clock = mock.clock(on)
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('session.cwd', async () => ({ value: '/work' }))
  on('process.spawn', async function* () {
    return { value: { code: 0, signal: null } }
  })
  on('process.run', async (_, e) => {
    if (e.argv[1] === 'pane-send') {
      sent.push(e.argv[2])
      await clock.sleep(50)
    }
    return { value: { exitCode: 0, stdout: '[]', stderr: '' } }
  })
  await $.command.run({ command: 'cellix-open', args: 'a.md' })
  await $.command.run({ command: 'cellix-open', args: 'b.md' })
  await $.command.run({ command: 'cellix-open', args: 'c.md' })
  await clock.advance(200)
  expect(sent.length).toBe(2)
  expect(JSON.parse(sent[1]).length).toBe(2)
})

test("when cellix can't be started, the pane says why instead of waiting", async ($, on) => {
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('process.run', async () => ({ value: { exitCode: 1, stdout: '', stderr: '' } }))
  on('env.get', async () => ({ value: undefined }))
  // eslint-disable-next-line require-yield
  on('process.spawn', async function* () {
    return { deny: 'spawn cellix ENOENT' }
  })
  await $.command.run({ command: 'cellix', args: '' })
  const pane = await $.ui.mount({
    plugin: 'cellix',
    surface: 'terminal',
    component: 'Pane',
    requestId: 'cellix',
    props: { bodyColumns: 80, scroll: { bodyRows: 20 } } as never,
  })
  expect(await pane.find({ type: 'Text', text: /can't find cellix on the PATH.*cellix setup claude/ })).toBeDefined()
  expect(await pane.find({ type: 'Text', text: /Starting Cellix/ })).toBeUndefined()
})

test('off Claude Code PATH, Cellix starts from where setup found it, and says which protocol it speaks', async ($, on) => {
  const started: { argv: readonly string[]; env?: Record<string, string> }[] = []
  const ran: string[] = []
  const clock = mock.clock(on)
  on('ui.open', async () => ({ value: { isPlaced: true as const } }))
  on('session.cwd', async () => ({ value: '/work' }))
  on('process.run', async (_, e) => {
    if (e.argv[0] === 'cellix') return { deny: 'spawn cellix ENOENT' }
    ran.push(e.argv.join(' '))
    return { value: { exitCode: 0, stdout: '[]', stderr: '' } }
  })
  on('env.get', async (_, e) => ({ value: e.name === 'CELLIX_BIN' ? '/opt/cellix/bin/cellix' : undefined }))
  on('process.spawn', async function* (_, e) {
    started.push({ argv: e.argv, env: e.env })
    if (e.argv[0] === 'cellix') return { deny: 'spawn cellix ENOENT' }
    yield { stream: 'stdout' as const, text: cellsLine('x') }
    return { value: { code: 0, signal: null } }
  })
  await $.command.run({ command: 'cellix', args: 'plan.md' })
  await clock.settle()
  // The open went before Cellix was found, so it tries again once it is.
  await clock.advance(100)
  await clock.settle()
  expect(started.map(s => s.argv[0])).toEqual(['cellix', '/opt/cellix/bin/cellix'])
  expect(started.every(s => s.env?.CELLIX_MOD === '2')).toBe(true)
  expect(ran).toContain('/opt/cellix/bin/cellix pane-send [{"type":"open","path":"/work/plan.md"}]')
})
