import { createAlertProcessor } from './alerts'
import { config, thresholds } from './config'
import { getHistory, pushHistory } from './event_history'
import { buildTickFrame, createCollectorState, errorFrame, recordBlock } from './frame'
import { readMetricsSnapshot } from './metric_snapshot'
import { fetchBlockNumber, fetchMetricsText } from './poll'
import { createTelegramNotifier, downTelegramMessage } from './telegram'
import type { Frame, Snapshot, Warning } from './types'

function downWarning(level: Warning['level'], key: string, short: string, detail: string): Warning {
  const w: Warning = { level, key, short, detail }
  return { ...w, notify: downTelegramMessage(w) }
}

const RPC_DOWN = downWarning('crit', 'RPC_DOWN', 'RPC DOWN', 'RPC endpoint not responding — node may be down or unreachable')
const METRICS_DOWN = downWarning('warn', 'METRICS_DOWN', 'METRICS DOWN', 'Metrics endpoint unavailable — node process may be down')
const DOWN_SNAP: Snapshot = {
  block: 0, blockDelta: null, bps: '?', lag: 0, peers: null, p99: null,
  rpcMs: 0, syncing: null, syncPct: null, uptime: '?', ver: '?',
}

// The data half of the old tick(): polls RPC + metrics, holds the previous-tick
// state needed for deltas, runs warning/Telegram logic, and returns a Frame.
// Stateless rendering is done elsewhere (render.ts) from the returned Frame.
export function createCollector() {
  const state = createCollectorState()
  const alerts = createAlertProcessor({
    delayTicks: config.telegram?.delay_ticks ?? 3,
    history: { push: pushHistory },
    notifier: createTelegramNotifier(config.telegram),
  })

  async function tick(): Promise<Frame> {
    const ts = new Date().toLocaleTimeString('en-GB', { hour12: false })

    let block: number
    let rpcMs: number
    try {
      const rpc = await fetchBlockNumber()
      block = rpc.block
      rpcMs = rpc.rpcMs
    } catch {
      // Node/RPC unreachable: raise a crit DOWN warning through the alert
      // processor (notifies after delay_ticks, resolves on recovery) and freeze
      // the other warnings since we can no longer observe them.
      const newCrit = alerts.processWarnings(ts, [ RPC_DOWN ], DOWN_SNAP, false)
      return { ...errorFrame(ts, getHistory(), 'rpc'), status: 'crit', newCrit }
    }

    const { blockDelta, bps } = recordBlock(state, block)

    let mt: string
    try {
      mt = await fetchMetricsText()
    } catch {
      alerts.processWarnings(ts, [ METRICS_DOWN ], DOWN_SNAP, false)
      return { ...errorFrame(ts, getHistory(), 'metrics', { block, blockDelta, bps }), status: 'warn' }
    }

    const built = buildTickFrame(state, {
      ts,
      block,
      blockDelta,
      bps,
      rpcMs,
      metrics: readMetricsSnapshot(mt),
      thresholds,
    })
    const newCrit = alerts.processWarnings(ts, built.frame.warnings, built.snap)
    if (built.restarted) alerts.notifyRestart(ts, built.uptime, built.snap)
    return { ...built.frame, newCrit, history: getHistory() }
  }

  return { tick }
}
