import type { ServerMsg } from './types'

export function encodeMsg(msg: ServerMsg): string {
  return JSON.stringify(msg) + '\n'
}

// Stateful newline decoder: feed raw socket chunks, get back complete messages.
// A single message may span chunks, and one chunk may hold several messages.
export function createDecoder() {
  let buf = ''
  return function push(chunk: string): ServerMsg[] {
    buf += chunk
    const out: ServerMsg[] = []
    let nl: number
    while ((nl = buf.indexOf('\n')) !== -1) {
      const line = buf.slice(0, nl)
      buf = buf.slice(nl + 1)
      if (line.trim()) {
        try { out.push(JSON.parse(line) as ServerMsg) } catch {}
      }
    }
    return out
  }
}
