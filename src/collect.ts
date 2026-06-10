import { createAlertProcessor } from './alerts'
import { config, thresholds } from './config'
import { getHistory, pushHistory } from './event_history'
import { buildTickFrame, createCollectorState, errorFrame, recordBlock } from './frame'
import { readMetricsSnapshot } from './metric_snapshot'
import { fetchBlockNumber, fetchMetricsText } from './poll'
import { createTelegramNotifier } from './telegram'
import type { Frame } from './types'

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
      pushHistory({ time: ts, level: 'error', text: 'RPC failed' })
      return errorFrame(ts, getHistory(), 'rpc')
    }

    const { blockDelta, bps } = recordBlock(state, block)

    let mt: string
    try {
      mt = await fetchMetricsText()
    } catch {
      pushHistory({ time: ts, level: 'warn', text: 'Metrics endpoint unavailable' })
      return errorFrame(ts, getHistory(), 'metrics', { block, blockDelta, bps })
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
