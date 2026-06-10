import { c } from './colors'
import type { HistoryEntry } from './types'

const EOL = '\x1b[K'

export function eventIcon(level: HistoryEntry['level']): string {
  return level === 'crit' ? c.red('●')
    : level === 'error' ? c.red('✗')
      : level === 'resolved' ? c.green('✓')
        : c.yellow('▲')
}

export function renderHistory(entries: HistoryEntry[]): string {
  if (entries.length === 0) return ''
  let out = `${EOL}\n`
  out += `${c.dim('Recent events:')}${EOL}\n`
  for (const e of entries) {
    out += `${c.dim(e.time)} ${eventIcon(e.level)} ${e.text}${EOL}\n`
  }
  return out
}
