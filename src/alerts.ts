import {
  resolvedTelegramMessage,
  restartTelegramMessage,
  type Notifier,
  warningTelegramMessage,
} from './telegram'
import type { HistoryStore } from './event_history'
import type { Snapshot, Thresholds, Warning } from './types'

function fmtDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m${s % 60}s`
  const h = Math.floor(m / 60)
  return `${h}h${m % 60}m`
}

export function checkWarnings(
  blockDelta: number | null,
  dv: number | null, dt: number | null, dtc: number | null,
  syncing: number | null, lagging: number,
  peers: number | null, latP99: number | null,
  rpcMs: number | null,
  thresholds: Thresholds,
): Warning[] {
  const w: Warning[] = []

  if (syncing === 1)
    w.push({ level: 'crit', key: 'SYNCING', short: 'SYNCING', detail: 'Node is state-syncing, not fully caught up with the network' })

  if (blockDelta !== null && blockDelta === 0)
    w.push({ level: 'crit', key: 'STUCK', short: 'STUCK', detail: 'Block number is not advancing — node may be stuck or disconnected' })

  if (dv !== null && dv === 0 && blockDelta !== null && blockDelta > 0)
    w.push({ level: 'crit', key: 'NOT_VOTING', short: 'NOT VOTING', detail: 'Node is not creating votes while blocks advance — consensus participation lost' })

  if (dt !== null && dt > 0)
    w.push({ level: 'warn', key: 'TMO', short: `TMO Δ${dt}`, detail: `${dt} new local timeout(s) — node failed to receive proposal in time` })

  if (dtc !== null && dtc > thresholds.tc_max)
    w.push({ level: 'warn', key: 'TC', short: `TC Δ${dtc}`, detail: `${dtc} new timeout certificate(s) — round(s) completed via timeout instead of QC` })

  if (lagging > thresholds.lag_max)
    w.push({ level: 'warn', key: 'LAG', short: `LAG ${lagging}`, detail: `Execution behind consensus by ${lagging} blocks` })

  if (peers !== null && peers < thresholds.peers_min)
    w.push({ level: 'warn', key: 'LOW_PEERS', short: `LOW PEERS ${peers}`, detail: `Only ${peers} peers discovered — network connectivity may be degraded` })

  if (latP99 !== null && latP99 > thresholds.p99_max)
    w.push({ level: 'warn', key: 'HIGH_P99', short: `HIGH P99 ${latP99}ms`, detail: `Broadcast latency p99 is ${latP99}ms — may miss round deadlines` })

  if (rpcMs !== null && rpcMs > thresholds.rpc_max)
    w.push({ level: 'warn', key: 'SLOW_RPC', short: `SLOW RPC ${rpcMs}ms`, detail: `RPC response took ${rpcMs}ms — node may be overloaded` })

  return w
}

export function createAlertProcessor(opts: {
  delayTicks: number
  history: HistoryStore
  notifier: Notifier
  now?: () => number
}) {
  const warningTicks = new Map<string, number>()
  const warningStart = new Map<string, number>()
  const telegramSent = new Set<string>()
  let prevWarnings = new Map<string, Warning>()
  const pendingResolves = new Map<string, { warning: Warning; dur: string; ticks: number }>()
  const now = opts.now ?? Date.now

  // `observable` is false when the node is unreachable (RPC/metrics down): we
  // can't tell whether the pre-existing warnings still hold, so we freeze them
  // (don't resolve, don't re-alert) and only process the injected DOWN warning.
  function processWarnings(ts: string, warnings: Warning[], snap: Snapshot, observable = true): boolean {
    const curWarnings = new Map(warnings.map(w => [ w.key, w ]))
    let newCrit = false
    const nowMs = now()

    for (const w of warnings) {
      const ticks = (warningTicks.get(w.key) ?? 0) + 1
      warningTicks.set(w.key, ticks)
      pendingResolves.delete(w.key)

      if (!prevWarnings.has(w.key)) {
        warningStart.set(w.key, nowMs)
        opts.history.push({ time: ts, level: w.level, text: w.detail })
        if (w.level === 'crit') newCrit = true
      }

      if (w.level === 'crit' && ticks >= opts.delayTicks && !telegramSent.has(w.key)) {
        telegramSent.add(w.key)
        opts.notifier.send(w.notify ?? warningTelegramMessage(w, snap), w.key)
      }
    }

    if (!observable) {
      // Keep prior warnings alive alongside the DOWN warning — no resolves while blind.
      for (const [ key, w ] of curWarnings) prevWarnings.set(key, w)
      return newCrit
    }

    for (const [ key, w ] of prevWarnings) {
      if (!curWarnings.has(key)) {
        warningTicks.delete(key)
        const started = warningStart.get(key)
        warningStart.delete(key)
        const dur = started ? ` (${fmtDuration(nowMs - started)})` : ''
        opts.history.push({ time: ts, level: 'resolved', text: `Resolved: ${w.short}${dur}` })
        if (telegramSent.has(key)) {
          pendingResolves.set(key, { warning: w, dur, ticks: 1 })
        }
      }
    }

    for (const [ key, pr ] of pendingResolves) {
      if (curWarnings.has(key)) {
        pendingResolves.delete(key)
        continue
      }
      pr.ticks++
      if (pr.ticks >= opts.delayTicks) {
        telegramSent.delete(key)
        opts.notifier.send(resolvedTelegramMessage(pr.warning, pr.dur, snap), `resolved:${key}`)
        pendingResolves.delete(key)
      }
    }

    prevWarnings = curWarnings
    return newCrit
  }

  function notifyRestart(ts: string, uptimeFmt: string, snap: Snapshot) {
    opts.history.push({ time: ts, level: 'warn', text: `Node restarted (uptime ${uptimeFmt})` })
    opts.notifier.send(restartTelegramMessage(uptimeFmt, snap), 'RESTART')
  }

  return { processWarnings, notifyRestart }
}
