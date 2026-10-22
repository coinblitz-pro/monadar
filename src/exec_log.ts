import { spawn, type ChildProcess } from 'child_process'
import type { ExecStats } from './types'

// Per-block execution latency from monad-execution's journal. Each executed
// block logs a line like:
//   2026-10-22 09:31:36.204673697 [118970] runloop_monad.cpp:381 LOG_INFO __exec_block,bl=115577020,id=0x..,ts=1792661496200,tx=12,...
// The leading timestamp (UTC, ns) is when execution finished; `ts` (ms) is when
// our BFT accepted the proposal. Both come from this host's clock, so their
// difference is the real execution latency — no metric export delay involved.

const WINDOW_MS = 60_000
const RESTART_DELAY_MS = 5_000
const LINE_RE = /^(\d{4})-(\d\d)-(\d\d) (\d\d):(\d\d):(\d\d)\.(\d+) .*__exec_block,bl=(\d+),.*?,ts=(\d+)/

export type ExecSample = { block: number; latencyMs: number; doneMs: number }
export type ExecSnapshot = { stats: ExecStats | null; lastBlock: number | null }

export function parseExecBlockLine(line: string): ExecSample | null {
  const m = LINE_RE.exec(line)
  if (!m) return null
  const [ , y, mo, d, h, mi, s, frac, bl, ts ] = m
  const fracMs = Number('0.' + frac) * 1000
  const doneMs = Date.UTC(+y, +mo - 1, +d, +h, +mi, +s) + fracMs
  return { block: Number(bl), latencyMs: doneMs - Number(ts), doneMs }
}

function percentile(sorted: number[], p: number): number {
  return sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))]
}

export function summarize(latencies: number[]): ExecStats | null {
  if (latencies.length === 0) return null
  const sorted = [ ...latencies ].sort((a, b) => a - b)
  return {
    count: sorted.length,
    p50: Math.round(percentile(sorted, 0.5)),
    p99: Math.round(percentile(sorted, 0.99)),
    max: Math.round(sorted[sorted.length - 1]),
  }
}

// Rolling window of samples keyed by execution finish time.
export function createExecWindow(windowMs = WINDOW_MS) {
  const samples: ExecSample[] = []
  let lastBlock: number | null = null

  function push(s: ExecSample) {
    samples.push(s)
    lastBlock = s.block
  }

  function snapshot(nowMs = Date.now()): ExecSnapshot {
    while (samples.length > 0 && samples[0].doneMs < nowMs - windowMs) samples.shift()
    // No recent lines (execution down, journal unreadable): report nothing so
    // the caller falls back to metrics instead of showing a stale block.
    if (samples.length === 0) return { stats: null, lastBlock: null }
    return { stats: summarize(samples.map(s => s.latencyMs)), lastBlock }
  }

  return { push, snapshot }
}

// Follows `journalctl -f` for monad-execution and feeds parsed lines into a
// window. Restarts journalctl if it exits. Needs root or the systemd-journal group.
export function startExecLogTail(window = createExecWindow(), unit = 'monad-execution') {
  let child: ChildProcess | null = null
  let stopped = false

  function start() {
    let buf = ''
    child = spawn('journalctl', [ '-f', '-n', '0', '-o', 'cat', '-u', unit ], { stdio: [ 'ignore', 'pipe', 'ignore' ] })
    child.stdout!.setEncoding('utf-8')
    child.stdout!.on('data', (chunk: string) => {
      buf += chunk
      let nl: number
      while ((nl = buf.indexOf('\n')) !== -1) {
        const s = parseExecBlockLine(buf.slice(0, nl))
        buf = buf.slice(nl + 1)
        if (s) window.push(s)
      }
    })
    child.on('error', () => {})
    child.on('exit', () => {
      child = null
      if (!stopped) setTimeout(start, RESTART_DELAY_MS).unref()
    })
  }

  start()
  process.on('exit', () => child?.kill())

  return {
    snapshot: window.snapshot,
    stop() {
      stopped = true
      child?.kill()
    },
  }
}
