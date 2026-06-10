import { checkWarnings, getHistory, notifyRestart, processWarnings, pushHistory } from './alerts'
import { METRICS_URL, RPC_URL } from './config'
import { delta, formatUptime } from './format'
import { metricLabel, metricValue } from './metrics'
import { BUILD_VERSION, type CounterCell, type Frame } from './protocol'
import type { Snapshot } from './types'

const RATE_WINDOW = 30

function cell(value: number | null, prev: number | null): CounterCell {
  return { value, delta: delta(value, prev) }
}

// Neutral frame used for error states and as a base for the full frame.
function blankFrame(ts: string): Frame {
  const z: CounterCell = { value: null, delta: null }
  return {
    ts,
    block: 0, blockDelta: null, bps: '?', ver: '?', uptime: '?',
    syncing: null, syncPct: null, peers: null, sessions: null,
    commits: z, votes: z, timeouts: z, tc: z, execLag: 0, lagEvtDelta: null,
    execBlock: null, tx: z, txpool: null, fwd: z, p99: null, rpcMs: 0,
    key: '?', network: '?', stakePct: '?', proposals: z, leaderPct: '?', votedPct: null,
    buildVersion: BUILD_VERSION, status: 'ok', newCrit: false, warnings: [], history: getHistory(), logLine: '',
  }
}

// The data half of the old tick(): polls RPC + metrics, holds the previous-tick
// state needed for deltas, runs warning/Telegram logic, and returns a Frame.
// Stateless rendering is done elsewhere (render.ts) from the returned Frame.
export function createCollector() {
  let prevBlock: number | null = null
  let prevCommits: number | null = null
  let prevVotes: number | null = null
  let prevTimeouts: number | null = null
  let prevFwd: number | null = null
  let prevLagging: number | null = null
  let prevTc: number | null = null
  let prevTxCommits: number | null = null
  let prevProposals: number | null = null
  let prevUptimeUs: number | null = null
  let execLag = 0

  const blockSamples: { time: number; block: number }[] = []

  function updateBlockRate(block: number): string {
    const now = Date.now()
    blockSamples.push({ time: now, block })
    if (blockSamples.length > RATE_WINDOW) blockSamples.shift()
    if (blockSamples.length < 2) return '?'
    const first = blockSamples[0]
    const last = blockSamples[blockSamples.length - 1]
    const dt = (last.time - first.time) / 1000
    if (dt === 0) return '?'
    return ((last.block - first.block) / dt).toFixed(2)
  }

  function shortKey(key: string): string {
    if (key.length <= 12) return key
    return key.slice(0, 6) + '..' + key.slice(-4)
  }

  async function tick(): Promise<Frame> {
    const ts = new Date().toLocaleTimeString('en-GB', { hour12: false })

    let blockDec: number
    let rpcMs: number
    try {
      const t0 = Date.now()
      const resp = await fetch(RPC_URL, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', method: 'eth_blockNumber', params: [], id: 1 }),
        signal: AbortSignal.timeout(5000),
      })
      const json = (await resp.json()) as { result: string }
      rpcMs = Date.now() - t0
      blockDec = parseInt(json.result, 16)
    } catch {
      pushHistory({ time: ts, level: 'error', text: 'RPC failed' })
      return { ...blankFrame(ts), error: 'rpc' }
    }

    const blockDeltaN = prevBlock !== null ? blockDec - prevBlock : null
    prevBlock = blockDec
    const bps = updateBlockRate(blockDec)

    let mt: string
    try {
      const resp = await fetch(METRICS_URL, { signal: AbortSignal.timeout(3000) })
      mt = await resp.text()
    } catch {
      pushHistory({ time: ts, level: 'warn', text: 'Metrics endpoint unavailable' })
      return { ...blankFrame(ts), error: 'metrics', block: blockDec, blockDelta: blockDeltaN, bps }
    }

    const v = (name: string) => metricValue(mt, name)

    // Node info
    const ver = metricLabel(mt, 'monad_node_info', 'service_version')
    const peers = v('monad_peer_disc_num_peers')
    const stakeBps = v('monad_state_node_state_self_stake_bps')
    const sessions = v('monad_wireauth_udp_state_total_sessions')
    const syncing = v('monad_statesync_syncing')
    const uptimeUs = v('monad_total_uptime_us')

    // Consensus
    const commits = v('monad_state_consensus_events_commit_block')
    const votes = v('monad_state_consensus_events_created_vote')
    const timeouts = v('monad_state_consensus_events_local_timeout')
    const tc = v('monad_state_consensus_events_created_tc')
    const lagging = v('monad_state_consensus_events_rx_execution_lagging')

    // Validator participation
    const proposals = v('monad_state_consensus_events_creating_proposal')
    const handled = v('monad_state_consensus_events_handle_proposal')

    // Execution
    const execBlock = v('monad_execution_ledger_block_num')
    const txCommits = v('monad_execution_ledger_num_tx_commits')

    // Txpool
    const txpool = v('monad_bft_txpool_pool_tracked_txs')
    const fwd = v('monad_bft_txpool_pool_insert_forwarded_txs')

    // Network
    const latP99 = v('monad_bft_raptorcast_udp_primary_broadcast_latency_p99_ms')

    // Statesync progress
    const syncProgress = v('monad_statesync_progress_estimate')
    const syncTarget = v('monad_statesync_last_target')

    const upFmt = uptimeUs !== null ? formatUptime(uptimeUs) : '?'
    const stakePct = stakeBps !== null ? (stakeBps / 100).toFixed(2) + '%' : '?'

    // Node restart: uptime is monotonic, so a drop means the process restarted.
    // All metric counters reset to 0 on the node, so clear the previous-tick
    // counters to neutralize this tick's deltas (avoids bogus alerts/negatives).
    const restarted = prevUptimeUs !== null && uptimeUs !== null && uptimeUs < prevUptimeUs
    prevUptimeUs = uptimeUs
    if (restarted) {
      prevCommits = prevVotes = prevTimeouts = prevTc = null
      prevLagging = prevTxCommits = prevProposals = prevFwd = null
    }

    const dv = delta(votes, prevVotes)
    const dt = delta(timeouts, prevTimeouts)
    const dtc = delta(tc, prevTc)
    const dl = delta(lagging, prevLagging)
    const dc = delta(commits, prevCommits)

    // Direct lag from execution ledger metric (preferred)
    if (execBlock !== null) {
      execLag = Math.max(0, Math.round(execBlock) - blockDec)
    } else if (dc !== null && blockDeltaN !== null) {
      // Fallback: computed lag from deltas
      execLag = Math.max(0, execLag + dc - blockDeltaN)
    }

    // Sync progress percentage
    let syncPct: string | null = null
    if (syncing === 1 && syncProgress !== null && syncTarget !== null && syncTarget > 0) {
      syncPct = ((syncProgress / syncTarget) * 100).toFixed(2)
    }

    const warnings = checkWarnings(blockDeltaN, dv, dt, dtc, syncing, execLag, peers, latP99, rpcMs)
    const hasCrit = warnings.some(w => w.level === 'crit')
    const hasWarn = warnings.some(w => w.level === 'warn')

    const snap: Snapshot = {
      block: blockDec, blockDelta: blockDeltaN, bps,
      lag: execLag, peers, p99: latP99, rpcMs,
      syncing, syncPct, uptime: upFmt, ver,
    }
    const newCrit = processWarnings(ts, warnings, snap)
    if (restarted) notifyRestart(ts, upFmt, snap)

    const nodeKey = metricLabel(mt, 'monad_node_info', 'secp_key')
    const network = metricLabel(mt, 'monad_node_info', 'network')

    let leaderPct = '?'
    if (proposals !== null && handled !== null && handled > 0) {
      leaderPct = (proposals / handled * 100).toFixed(2) + '%'
    }
    const votedPct = votes !== null && handled !== null && handled > 0
      ? votes / handled * 100
      : null

    // Log line — identical key=val format as before; persisted by the caller.
    const iso = new Date().toISOString()
    const warnTags = warnings.length > 0 ? ' WARN=' + warnings.map(w => w.short).join(',') : ''
    const eb = execBlock !== null ? Math.round(execBlock) : '?'
    const votedLog = votedPct !== null ? votedPct.toFixed(1) + '%' : '?'
    const logLine = [
      iso,
      `block=${blockDec}`, `bps=${bps}`,
      `ver=${ver}`, `up=${upFmt}`,
      `peers=${peers ?? '?'}`, `stake=${stakePct}`, `sessions=${sessions ?? '?'}`, `sync=${syncing === 1 ? 'yes' : 'no'}`,
      ...(syncPct !== null ? [`sync_pct=${syncPct}%`] : []),
      `commits=${commits ?? '?'}`, `votes=${votes ?? '?'}`, `tmo=${timeouts ?? '?'}`, `tc=${tc ?? '?'}`,
      `lag=${execLag}`, `lag_evt=${lagging ?? 0}`,
      `exec=${eb}`, `tx_commits=${txCommits ?? '?'}`,
      `txpool=${txpool ?? '?'}`, `fwd=${fwd ?? '?'}`, `p99=${latP99 ?? '?'}ms`,
      `rpc=${rpcMs}ms`,
      `key=${shortKey(nodeKey)}`, `net=${network}`,
      ...(proposals !== null ? [`prop=${proposals}`] : []),
      ...(handled !== null ? [`handled=${handled}`] : []),
      `leader=${leaderPct}`, `voted=${votedLog}`,
    ].join(' ') + warnTags

    const frame: Frame = {
      ts,
      block: blockDec, blockDelta: blockDeltaN, bps, ver, uptime: upFmt,
      syncing, syncPct, peers, sessions,
      commits: cell(commits, prevCommits),
      votes: cell(votes, prevVotes),
      timeouts: cell(timeouts, prevTimeouts),
      tc: cell(tc, prevTc),
      execLag, lagEvtDelta: dl,
      execBlock: execBlock !== null ? Math.round(execBlock) : null,
      tx: cell(txCommits, prevTxCommits),
      txpool, fwd: cell(fwd, prevFwd),
      p99: latP99, rpcMs,
      key: shortKey(nodeKey), network, stakePct,
      proposals: cell(proposals, prevProposals), leaderPct, votedPct,
      buildVersion: BUILD_VERSION,
      status: hasCrit ? 'crit' : hasWarn ? 'warn' : 'ok',
      newCrit,
      warnings, history: getHistory(), logLine,
    }

    prevCommits = commits ?? prevCommits
    prevVotes = votes ?? prevVotes
    prevTimeouts = timeouts ?? prevTimeouts
    prevFwd = fwd ?? prevFwd
    prevLagging = lagging ?? prevLagging
    prevTc = tc ?? prevTc
    prevTxCommits = txCommits ?? prevTxCommits
    prevProposals = proposals ?? prevProposals

    return frame
  }

  return { tick }
}
