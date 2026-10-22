import { sizeRatio } from './bft_log'
import type { Config, Frame, RoundAgg, RoundSummary, Snapshot, Warning } from './types'

export type Notifier = {
  send(text: string, key: string): void
}

function html(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
}

function fmtSnap(s: Snapshot): string {
  const lines = [
    `block  ${s.block} Δ${s.blockDelta ?? '?'} (${s.bps}/s)`,
    `lag    ${s.lag}`,
    `exec   ${s.execP99 !== null ? `p99 ${s.execP99}ms` : '?'}`,
    `rounds ${s.led24h !== null ? `${s.led24h} led, ${s.missed24h} missed (24h)` : '?'}`,
    `peers  ${s.peers ?? '?'}`,
    `p99    ${s.p99 ?? '?'}ms`,
    `rpc    ${s.rpcMs}ms`,
    `sync   ${s.syncing === 1 ? `yes${s.syncPct ? ` ${s.syncPct}%` : ''}` : 'no'}`,
    `up     ${s.uptime}`,
    `ver    ${s.ver}`,
  ]
  return '<pre>' + html(lines.join('\n')) + '</pre>'
}

export function warningTelegramMessage(w: Warning, snap: Snapshot): string {
  return `🚨 <b>${html(w.short)}</b>\n${html(w.detail)}\n${fmtSnap(snap)}`
}

export function resolvedTelegramMessage(w: Warning, dur: string, snap: Snapshot): string {
  return `✅ <b>${html(w.short)}</b> resolved${html(dur)}\n${fmtSnap(snap)}`
}

export function restartTelegramMessage(uptimeFmt: string, snap: Snapshot): string {
  return `🔄 <b>NODE RESTART</b>\nuptime ${html(uptimeFmt)}\n${fmtSnap(snap)}`
}

// Alert body for an endpoint-down warning — no snapshot, since we have no data.
export function downTelegramMessage(w: Warning): string {
  return `🚨 <b>${html(w.short)}</b>\n${html(w.detail)}`
}

function roundLines(r: RoundSummary): string[] {
  const line = (label: string, a: RoundAgg) =>
    `${label} led ${a.led}, ok ${a.ok}, missed ${a.missed + a.failed}, unseen ${a.notReceived}, late ${a.lateTmo}, blocks ${a.committed}, txs ${a.txs}`
  const net = r.h24.netRounds > 0 ? (r.h24.netMissed / r.h24.netRounds * 100).toFixed(2) : '?'
  const size = (a: RoundAgg) => {
    const ratio = sizeRatio(a)
    return ratio === null ? '?' : `${Math.round(ratio * 100)}% (${a.sizeSamples} blocks)`
  }
  return [
    line('1h   ', r.h1),
    line('24h  ', r.h24),
    `size   our blocks vs network: 1h ${size(r.h1)}, 24h ${size(r.h24)}`,
    `fwd    txs forwarded to us: 1h ${r.h1.fwdTxs}, 24h ${r.h24.fwdTxs}`,
    `net    ${r.roundsPerSec.toFixed(2)} rounds/s, TC ${r.h24.netMissed}/${r.h24.netRounds} (${net}%) 24h`,
    ...(r.lastMissed ? [ `last   missed round ${r.lastMissed.round} at ${new Date(r.lastMissed.t).toISOString().slice(0, 19)}Z` ] : []),
  ]
}

function statusIcon(s: Frame['status']): string {
  return s === 'crit' ? '🔴' : s === 'warn' ? '🟡' : '🟢'
}

// On-demand status reply for the Telegram bot's /status command.
export function frameStatusMessage(f: Frame): string {
  if (f.error) return `⚠️ <b>monadar</b> — ${f.error.toUpperCase()} error`
  const lines = [
    `block  ${f.block} Δ${f.blockDelta ?? '?'} (${f.bps}/s)`,
    `lag    ${f.execLag}`,
    `exec   ${f.exec ? `p50 ${f.exec.p50}ms p99 ${f.exec.p99}ms max ${f.exec.max}ms` : '?'}`,
    ...(f.rounds ? roundLines(f.rounds) : [ `rounds ?` ]),
    `peers  ${f.peers ?? '?'}`,
    `p99    ${f.p99 ?? '?'}ms`,
    `rpc    ${f.rpcMs}ms`,
    `sync   ${f.syncing === 1 ? `yes${f.syncPct ? ` ${f.syncPct}%` : ''}` : 'no'}`,
    `up     ${f.uptime}`,
    `ver    ${f.ver}`,
  ]
  const head = `${statusIcon(f.status)} <b>monadar</b> — ${f.status.toUpperCase()}  <i>${html(f.ts)}</i>`
  const warns = f.warnings.length ? '\n' + f.warnings.map(w => `• ${html(w.short)}`).join('\n') : ''
  return `${head}\n<pre>${html(lines.join('\n'))}</pre>${warns}`
}

// Recent-events reply for the Telegram bot's /events command.
export function frameEventsMessage(f: Frame): string {
  if (!f.history.length) return 'No recent events.'
  const rows = f.history.map(e => {
    const icon = e.level === 'resolved' ? '✓' : e.level === 'crit' || e.level === 'error' ? '✗' : '▲'
    return `${e.time} ${icon} ${e.text}`
  })
  return `<b>Recent events</b>\n<pre>${html(rows.join('\n'))}</pre>`
}

export function createTelegramNotifier(telegram: Config['telegram'] | undefined): Notifier {
  if (!telegram?.token || !telegram.chat_id) return { send: () => {} }

  const cooldown = new Map<string, number>()
  const cooldownMs = (telegram.cooldown_min ?? 5) * 60 * 1000

  return {
    send(text: string, key: string) {
      const now = Date.now()
      const last = cooldown.get(key) ?? 0
      if (now - last < cooldownMs) return
      cooldown.set(key, now)
      fetch(`https://api.telegram.org/bot${telegram.token}/sendMessage`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ chat_id: telegram.chat_id, text, parse_mode: 'HTML' }),
        signal: AbortSignal.timeout(5000),
      }).catch(() => {})
    },
  }
}
