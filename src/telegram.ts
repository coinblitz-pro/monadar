import type { Config, Snapshot, Warning } from './types'

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
