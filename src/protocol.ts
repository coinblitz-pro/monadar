import { existsSync } from 'fs'
import { dirname } from 'path'
import type { HistoryEntry, Warning } from './types'

declare const __BUILD_HASH__: string | undefined
export const BUILD_VERSION = typeof __BUILD_HASH__ !== 'undefined' ? __BUILD_HASH__ : 'dev'

// A counter metric carried as both its current value and the per-tick delta,
// so the renderer can show "value Δdelta" without holding previous state.
export type CounterCell = { value: number | null; delta: number | null }

// Wire snapshot broadcast by the daemon every tick. Contains raw numbers and
// deltas only (no ANSI) — the client applies colors from its own thresholds.
export type Frame = {
  ts: string
  error?: 'rpc' | 'metrics'
  // row1
  block: number
  blockDelta: number | null
  bps: string
  ver: string
  uptime: string
  syncing: number | null
  syncPct: string | null
  peers: number | null
  sessions: number | null
  // row2
  commits: CounterCell
  votes: CounterCell
  timeouts: CounterCell
  tc: CounterCell
  execLag: number
  lagEvtDelta: number | null
  // row3
  execBlock: number | null
  tx: CounterCell
  txpool: number | null
  fwd: CounterCell
  p99: number | null
  rpcMs: number
  // row4
  key: string
  network: string
  stakePct: string
  proposals: CounterCell
  leaderPct: string
  votedPct: number | null
  // meta
  buildVersion: string
  status: 'crit' | 'warn' | 'ok'
  newCrit: boolean
  warnings: Warning[]
  history: HistoryEntry[]
  logLine: string
}

export type ServerMsg =
  | { type: 'hello'; backlog: string[]; frame: Frame | null }
  | { type: 'snapshot'; frame: Frame }

const DEFAULT_SOCKET = '/run/monadar.sock'

let socketOverride: string | null = null

export function setSocketPath(p: string | null) {
  socketOverride = p
}

// Resolve the unix socket path. Order: --socket/config override → /run (if its
// parent dir is writable, i.e. root daemon) → $XDG_RUNTIME_DIR/tmp fallback.
export function socketPath(): string {
  if (socketOverride) return socketOverride
  const runDir = dirname(DEFAULT_SOCKET)
  if (existsSync(runDir)) return DEFAULT_SOCKET
  const base = process.env.XDG_RUNTIME_DIR || '/tmp'
  return `${base}/monadar.sock`
}

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
