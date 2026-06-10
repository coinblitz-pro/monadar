import { c } from './colors'
import { config, thresholds } from './config'
import { loadEvents, persistEvent } from './events'
import type { HistoryEntry, Snapshot, Warning } from './types'

const MAX_HISTORY = 5
const warningHistory: HistoryEntry[] = []

export function pushHistory(entry: HistoryEntry) {
  warningHistory.push(entry)
  if (warningHistory.length > MAX_HISTORY) warningHistory.shift()
  persistEvent(entry)
}

// Restore the recent-events strip from the persisted file on daemon startup so
// a restart doesn't blank it. Seeds in-memory only (no re-persist).
export function seedHistory() {
  for (const e of loadEvents().slice(-MAX_HISTORY)) {
    warningHistory.push({ time: e.at.slice(11, 19), level: e.level, text: e.text })
  }
  if (warningHistory.length > MAX_HISTORY) {
    warningHistory.splice(0, warningHistory.length - MAX_HISTORY)
  }
}

// Snapshot of the current event history (carried in each Frame to the client).
export function getHistory(): HistoryEntry[] {
  return warningHistory.slice()
}

export function renderHistory(entries: HistoryEntry[]): string {
  const eol = '\x1b[K'
  if (entries.length === 0) return ''
  let out = `${eol}\n`
  out += `${c.dim('Recent events:')}${eol}\n`
  for (const e of entries) {
    const icon = e.level === 'crit' ? c.red('●')
      : e.level === 'error' ? c.red('✗')
        : e.level === 'resolved' ? c.green('✓')
          : c.yellow('▲')
    out += `${c.dim(e.time)} ${icon} ${e.text}${eol}\n`
  }
  return out
}

const telegramCooldown = new Map<string, number>()
const TELEGRAM_COOLDOWN_MS = (config.telegram?.cooldown_min ?? 5) * 60 * 1000
const TELEGRAM_DELAY_TICKS = config.telegram?.delay_ticks ?? 3
const warningTicks = new Map<string, number>()
const warningStart = new Map<string, number>()
const telegramSent = new Set<string>()

function fmtSnap(s: Snapshot): string {
  const lines = [
    `block  ${s.block} Δ${s.blockDelta ?? '?'} (${s.bps}/s)`,
    `lag    ${s.lag}`,
    `peers  ${s.peers ?? '?'}`,
    `p99    ${s.p99 ?? '?'}ms`,
    `rpc    ${s.rpcMs}ms`,
    `sync   ${s.syncing === 1 ? `yes${s.syncPct ? ` ${s.syncPct}%` : ''}` : 'no'}`,
    `up     ${s.uptime}`,
    `ver    ${s.ver}`,
  ]
  return '<pre>' + lines.join('\n') + '</pre>'
}

function fmtDuration(ms: number): string {
  const s = Math.floor(ms / 1000)
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m${s % 60}s`
  const h = Math.floor(m / 60)
  return `${h}h${m % 60}m`
}

async function sendTelegram(text: string, key: string) {
  if (!config.telegram) return
  const now = Date.now()
  const last = telegramCooldown.get(key) ?? 0
  if (now - last < TELEGRAM_COOLDOWN_MS) return
  telegramCooldown.set(key, now)
  try {
    await fetch(`https://api.telegram.org/bot${config.telegram.token}/sendMessage`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ chat_id: config.telegram.chat_id, text, parse_mode: 'HTML' }),
      signal: AbortSignal.timeout(5000),
    })
  } catch {}
}

// Discrete node-restart event (detected via an uptime drop in collect.ts):
// record it in the history and fire a one-off Telegram, cooldown-guarded so a
// flapping node doesn't spam.
export function notifyRestart(ts: string, uptimeFmt: string, snap: Snapshot) {
  pushHistory({ time: ts, level: 'warn', text: `Node restarted (uptime ${uptimeFmt})` })
  const msg = `🔄 <b>NODE RESTART</b>\nuptime ${uptimeFmt}\n${fmtSnap(snap)}`
  sendTelegram(msg, 'RESTART')
}

export function checkWarnings(
  blockDelta: number | null,
  dv: number | null, dt: number | null, dtc: number | null,
  syncing: number | null, lagging: number,
  peers: number | null, latP99: number | null,
  rpcMs: number | null,
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

let prevWarnings = new Map<string, Warning>()
const pendingResolves = new Map<string, { warning: Warning, dur: string, ticks: number }>()

export function processWarnings(ts: string, warnings: Warning[], snap: Snapshot): boolean {
  const curWarnings = new Map(warnings.map(w => [ w.key, w ]))
  let newCrit = false
  const now = Date.now()

  for (const w of warnings) {
    const ticks = (warningTicks.get(w.key) ?? 0) + 1
    warningTicks.set(w.key, ticks)

    // Cancel pending resolve if warning reappears
    pendingResolves.delete(w.key)

    if (!prevWarnings.has(w.key)) {
      warningStart.set(w.key, now)
      pushHistory({ time: ts, level: w.level, text: w.detail })
      if (w.level === 'crit') newCrit = true
    }

    if (w.level === 'crit' && ticks >= TELEGRAM_DELAY_TICKS && !telegramSent.has(w.key)) {
      telegramSent.add(w.key)
      const msg = `🚨 <b>${w.short}</b>\n${w.detail}\n${fmtSnap(snap)}`
      sendTelegram(msg, w.key)
    }
  }

  // Newly resolved → push history immediately, defer Telegram
  for (const [ key, w ] of prevWarnings) {
    if (!curWarnings.has(key)) {
      warningTicks.delete(key)
      const started = warningStart.get(key)
      warningStart.delete(key)
      const dur = started ? ` (${fmtDuration(now - started)})` : ''
      pushHistory({ time: ts, level: 'resolved', text: `Resolved: ${w.short}${dur}` })
      if (telegramSent.has(key)) {
        pendingResolves.set(key, { warning: w, dur, ticks: 1 })
      }
    }
  }

  // Send resolved Telegram only after stable for DELAY_TICKS
  for (const [ key, pr ] of pendingResolves) {
    if (curWarnings.has(key)) {
      pendingResolves.delete(key)
      continue
    }
    pr.ticks++
    if (pr.ticks >= TELEGRAM_DELAY_TICKS) {
      telegramSent.delete(key)
      const msg = `✅ <b>${pr.warning.short}</b> resolved${pr.dur}\n${fmtSnap(snap)}`
      sendTelegram(msg, `resolved:${key}`)
      pendingResolves.delete(key)
    }
  }

  prevWarnings = curWarnings
  return newCrit
}
