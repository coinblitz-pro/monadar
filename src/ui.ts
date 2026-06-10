import { verbose } from './config'

export type Overlay = 'none' | 'help' | 'log' | 'events'

// Shared UI state, mutated by keypresses, read by the renderer.
export const ui = {
  overlay: (verbose ? 'help' : 'none') as Overlay,
}

function toggle(o: Overlay) {
  ui.overlay = ui.overlay === o ? 'none' : o
}

// Wire stdin in raw mode so single keypresses toggle overlays instantly.
// onRepaint redraws the last frame; onQuit performs graceful shutdown.
// Keys are matched by control-code so the source stays free of literal bytes.
export function setupInput(onRepaint: () => void, onQuit: () => void) {
  const stdin = process.stdin
  if (!stdin.isTTY) return
  stdin.setRawMode(true)
  stdin.resume()
  stdin.setEncoding('utf8')
  stdin.on('data', (key: string) => {
    switch (key.charCodeAt(0)) {
      case 3: onQuit(); return            // ctrl+c
      case 15: toggle('log'); break       // ctrl+o
      case 5: toggle('events'); break     // ctrl+e
      case 8: toggle('help'); break       // ctrl+h
      case 27: ui.overlay = 'none'; break // esc
      default: return
    }
    onRepaint()
  })
}
