import type { ClientModule } from 'claude-code'

type Input = Record<string, unknown>
type State = { queue: Input[] }

/** Catches the pane's keys and pointer. A post is replaced if another comes in the same
 * frame, so input queues here and goes in one post per tick, a run of moves as its last. */
const PaneInput: ClientModule<unknown, State> = (_props, surface) => {
  if (surface.state === undefined) {
    const state: State = { queue: [] }
    surface.setState(state)
    surface.onKey(e => {
      state.queue.push({ type: 'key', key: e.key, ctrl: e.ctrl, shift: e.shift, meta: e.meta })
    })
    surface.onPointer(e => {
      if (e.type === 'enter' || e.type === 'leave') return
      // Only where the pointer ended up matters, so a move replaces the one before it.
      const last = state.queue[state.queue.length - 1]
      if (e.type === 'move' && last?.kind === 'move' && last.button === e.button) state.queue.pop()
      state.queue.push({
        type: 'pointer',
        kind: e.type,
        x: e.x,
        y: e.y,
        fine: e.fine?.x,
        button: e.button,
      })
    })
    surface.every(16, () => {
      if (state.queue.length > 0) {
        surface.post(state.queue.splice(0) as never)
      }
    })
  }
  return surface.elements.Box({ width: surface.columns, height: surface.rows })
}

export default PaneInput
