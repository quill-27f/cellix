import type { Engine, Register } from 'claude-code'

const REFRESH_MS = 5000

type Comment = { file: string; line: number; text: string }

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

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'cellix-comments',
      description: 'List the review comments waiting in Cellix',
    })
    await $.command.register({
      name: 'cellix-open',
      description: 'Open a file in Cellix',
    })
    $.clock.every(REFRESH_MS, () => showCount($))
    return next(e)
  })

  on('command.run', { command: 'cellix-comments' }, async $ => {
    const comments = await pending($)
    if (comments === undefined) return { text: 'Cellix is not running in this folder.' }
    if (comments.length === 0) return { text: 'No comments are waiting.' }
    return { text: comments.map(c => `${c.file}:${c.line}: ${c.text}`).join('\n') }
  })

  on('command.run', { command: 'cellix-open' }, async ($, e) => {
    const { exitCode } = await $.process.run(['cellix', 'open', e.args])
    const opened = exitCode === 0
    return { text: opened ? `Opened ${e.args} in Cellix.` : 'Cellix is not running in this folder.' }
  })
}
