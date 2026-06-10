import { checkWarnings } from './alerts'
import { BUILD_VERSION } from './build'
import { delta, formatUptime } from './format'
import type { MetricsSnapshot } from './metric_snapshot'
import type { CounterCell, Frame, HistoryEntry, Snapshot, Thresholds } from './types'

const RATE_WINDOW = 30

export type CollectorState = {
  prevBlock: number | null
  prevCommits: number | null
  prevVotes: number | null
  prevTimeouts: number | null
  prevFwd: number | null
  prevLagging: number | null
  prevTc: number | null
  prevTxCommits: number | null
  prevProposals: number | null
  prevUptimeUs: number | null
  execLag: number
  blockSamples: { time: number; block: number }[]
}

export type BlockSample = {
  blockDelta: number | null
  bps: string
}

export type FrameDraft = Omit<Frame, 'newCrit' | 'history'>

export type BuiltFrame = {
  frame: FrameDraft
  snap: Snapshot
  restarted: boolean
  uptime: string
}

export function createCollectorState(): CollectorState {
  return {
    prevBlock: null,
    prevCommits: null,
    prevVotes: null,
    prevTimeouts: null,
    prevFwd: null,
    prevLagging: null,
    prevTc: null,
    prevTxCommits: null,
    prevProposals: null,
    prevUptimeUs: null,
    execLag: 0,
    blockSamples: [],
  }
}

function counter(value: number | null, prev: number | null): CounterCell {
  return { value, delta: delta(value, prev) }
}

export function shortKey(key: string): string {
  if (key.length <= 12) return key
  return key.slice(0, 6) + '..' + key.slice(-4)
}

export function recordBlock(state: CollectorState, block: number, nowMs = Date.now()): BlockSample {
  const blockDelta = state.prevBlock !== null ? block - state.prevBlock : null
  state.prevBlock = block

  state.blockSamples.push({ time: nowMs, block })
  if (state.blockSamples.length > RATE_WINDOW) state.blockSamples.shift()
  if (state.blockSamples.length < 2) return { blockDelta, bps: '?' }

  const first = state.blockSamples[0]
  const last = state.blockSamples[state.blockSamples.length - 1]
  const dt = (last.time - first.time) / 1000
  if (dt === 0) return { blockDelta, bps: '?' }

  return { blockDelta, bps: ((last.block - first.block) / dt).toFixed(2) }
}

function resetNodeCounters(state: CollectorState) {
  state.prevCommits = null
  state.prevVotes = null
  state.prevTimeouts = null
  state.prevTc = null
  state.prevLagging = null
  state.prevTxCommits = null
  state.prevProposals = null
  state.prevFwd = null
}

function logLine(input: {
  block: number
  bps: string
  rpcMs: number
  metrics: MetricsSnapshot
  uptime: string
  stakePct: string
  syncPct: string | null
  execLag: number
  execBlock: number | null
  leaderPct: string
  votedPct: number | null
  warningShorts: string[]
}): string {
  const m = input.metrics
  const warnTags = input.warningShorts.length > 0 ? ' WARN=' + input.warningShorts.join(',') : ''
  const execBlock = input.execBlock !== null ? input.execBlock : '?'
  const votedLog = input.votedPct !== null ? input.votedPct.toFixed(1) + '%' : '?'
  return [
    new Date().toISOString(),
    `block=${input.block}`, `bps=${input.bps}`,
    `ver=${m.ver}`, `up=${input.uptime}`,
    `peers=${m.peers ?? '?'}`, `stake=${input.stakePct}`, `sessions=${m.sessions ?? '?'}`,
    `sync=${m.syncing === 1 ? 'yes' : 'no'}`,
    ...(input.syncPct !== null ? [`sync_pct=${input.syncPct}%`] : []),
    `commits=${m.commits ?? '?'}`, `votes=${m.votes ?? '?'}`, `tmo=${m.timeouts ?? '?'}`, `tc=${m.tc ?? '?'}`,
    `lag=${input.execLag}`, `lag_evt=${m.lagging ?? 0}`,
    `exec=${execBlock}`, `tx_commits=${m.txCommits ?? '?'}`,
    `txpool=${m.txpool ?? '?'}`, `fwd=${m.fwd ?? '?'}`, `p99=${m.latP99 ?? '?'}ms`,
    `rpc=${input.rpcMs}ms`,
    `key=${shortKey(m.nodeKey)}`, `net=${m.network}`,
    ...(m.proposals !== null ? [`prop=${m.proposals}`] : []),
    ...(m.handled !== null ? [`handled=${m.handled}`] : []),
    `leader=${input.leaderPct}`, `voted=${votedLog}`,
  ].join(' ') + warnTags
}

export function buildTickFrame(
  state: CollectorState,
  input: {
    ts: string
    block: number
    blockDelta: number | null
    bps: string
    rpcMs: number
    metrics: MetricsSnapshot
    thresholds: Thresholds
  },
): BuiltFrame {
  const m = input.metrics
  const uptime = m.uptimeUs !== null ? formatUptime(m.uptimeUs) : '?'
  const stakePct = m.stakeBps !== null ? (m.stakeBps / 100).toFixed(2) + '%' : '?'

  const restarted = state.prevUptimeUs !== null && m.uptimeUs !== null && m.uptimeUs < state.prevUptimeUs
  state.prevUptimeUs = m.uptimeUs
  if (restarted) resetNodeCounters(state)

  const dv = delta(m.votes, state.prevVotes)
  const dt = delta(m.timeouts, state.prevTimeouts)
  const dtc = delta(m.tc, state.prevTc)
  const dl = delta(m.lagging, state.prevLagging)
  const dc = delta(m.commits, state.prevCommits)

  const execBlock = m.execBlock !== null ? Math.round(m.execBlock) : null
  let execLag: number
  if (execBlock !== null) {
    execLag = Math.max(0, input.block - execBlock)
  } else if (dc !== null && input.blockDelta !== null) {
    execLag = Math.max(0, state.execLag + input.blockDelta - dc)
  } else {
    execLag = state.execLag
  }
  state.execLag = execLag

  let syncPct: string | null = null
  if (m.syncing === 1 && m.syncProgress !== null && m.syncTarget !== null && m.syncTarget > 0) {
    syncPct = ((m.syncProgress / m.syncTarget) * 100).toFixed(2)
  }

  const warnings = checkWarnings(input.blockDelta, dv, dt, dtc, m.syncing, execLag, m.peers, m.latP99, input.rpcMs, input.thresholds)
  const hasCrit = warnings.some(w => w.level === 'crit')
  const hasWarn = warnings.some(w => w.level === 'warn')

  const snap: Snapshot = {
    block: input.block,
    blockDelta: input.blockDelta,
    bps: input.bps,
    lag: execLag,
    peers: m.peers,
    p99: m.latP99,
    rpcMs: input.rpcMs,
    syncing: m.syncing,
    syncPct,
    uptime,
    ver: m.ver,
  }

  let leaderPct = '?'
  if (m.proposals !== null && m.handled !== null && m.handled > 0) {
    leaderPct = (m.proposals / m.handled * 100).toFixed(2) + '%'
  }
  const votedPct = m.votes !== null && m.handled !== null && m.handled > 0
    ? m.votes / m.handled * 100
    : null

  const frame: FrameDraft = {
    ts: input.ts,
    block: input.block,
    blockDelta: input.blockDelta,
    bps: input.bps,
    ver: m.ver,
    uptime,
    syncing: m.syncing,
    syncPct,
    peers: m.peers,
    sessions: m.sessions,
    commits: counter(m.commits, state.prevCommits),
    votes: counter(m.votes, state.prevVotes),
    timeouts: counter(m.timeouts, state.prevTimeouts),
    tc: counter(m.tc, state.prevTc),
    execLag,
    lagEvtDelta: dl,
    execBlock,
    tx: counter(m.txCommits, state.prevTxCommits),
    txpool: m.txpool,
    fwd: counter(m.fwd, state.prevFwd),
    p99: m.latP99,
    rpcMs: input.rpcMs,
    key: shortKey(m.nodeKey),
    network: m.network,
    stakePct,
    proposals: counter(m.proposals, state.prevProposals),
    leaderPct,
    votedPct,
    buildVersion: BUILD_VERSION,
    status: hasCrit ? 'crit' : hasWarn ? 'warn' : 'ok',
    warnings,
    logLine: logLine({
      block: input.block,
      bps: input.bps,
      rpcMs: input.rpcMs,
      metrics: m,
      uptime,
      stakePct,
      syncPct,
      execLag,
      execBlock,
      leaderPct,
      votedPct,
      warningShorts: warnings.map(w => w.short),
    }),
  }

  state.prevCommits = m.commits ?? state.prevCommits
  state.prevVotes = m.votes ?? state.prevVotes
  state.prevTimeouts = m.timeouts ?? state.prevTimeouts
  state.prevFwd = m.fwd ?? state.prevFwd
  state.prevLagging = m.lagging ?? state.prevLagging
  state.prevTc = m.tc ?? state.prevTc
  state.prevTxCommits = m.txCommits ?? state.prevTxCommits
  state.prevProposals = m.proposals ?? state.prevProposals

  return { frame, snap, restarted, uptime }
}

export function errorFrame(
  ts: string,
  history: HistoryEntry[],
  error: 'rpc' | 'metrics',
  blockSample?: { block: number; blockDelta: number | null; bps: string },
): Frame {
  const z = (): CounterCell => ({ value: null, delta: null })
  return {
    ts,
    error,
    block: blockSample?.block ?? 0,
    blockDelta: blockSample?.blockDelta ?? null,
    bps: blockSample?.bps ?? '?',
    ver: '?',
    uptime: '?',
    syncing: null,
    syncPct: null,
    peers: null,
    sessions: null,
    commits: z(),
    votes: z(),
    timeouts: z(),
    tc: z(),
    execLag: 0,
    lagEvtDelta: null,
    execBlock: null,
    tx: z(),
    txpool: null,
    fwd: z(),
    p99: null,
    rpcMs: 0,
    key: '?',
    network: '?',
    stakePct: '?',
    proposals: z(),
    leaderPct: '?',
    votedPct: null,
    buildVersion: BUILD_VERSION,
    status: 'ok',
    newCrit: false,
    warnings: [],
    history,
    logLine: '',
  }
}
