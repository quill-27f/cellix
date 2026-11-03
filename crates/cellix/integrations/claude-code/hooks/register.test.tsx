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
  expect(sent).toContain('cellix pane-send {"type":"open","path":"/work/docs/plan.md"}')
})
