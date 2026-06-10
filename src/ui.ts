import { verbose } from './config'

export type Overlay = 'none' | 'help' | 'log'

// Shared UI state, mutated by keypresses, read by the renderer.
export const ui = {
  overlay: (verbose ? 'help' : 'none') as Overlay,
}

function toggle(o: Overlay) {
  ui.overlay = ui.overlay === o ? 'none' : o
}

// Wire stdin in raw mode so single keypresses toggle overlays instantly.
// onRepaint redraws the last frame; onQuit performs graceful shutdown.
export function setupInput(onRepaint: () => void, onQuit: () => void) {
  const stdin = process.stdin
  if (!stdin.isTTY) return
  stdin.setRawMode(true)
  stdin.resume()
  stdin.setEncoding('utf8')
  stdin.on('data', (key: string) => {
    switch (key) {
      case '': onQuit(); return           // ctrl+c
      case '': toggle('log'); break        // ctrl+o
      case '': toggle('help'); break       // ctrl+h
      case '': ui.overlay = 'none'; break  // esc
      default: return
    }
    onRepaint()
  })
}
