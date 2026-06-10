import type { Cell } from './types'

export function formatUptime(us: number): string {
  const s = Math.floor(us / 1_000_000)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (h >= 24) {
    const d = Math.floor(h / 24)
    return `${d}d${h % 24}h`
  }
  return `${h}h${m}m`
}

export function delta(cur: number | null, prev: number | null): number | null {
  if (cur === null || prev === null) return null
  return cur - prev
}

export function fmtDelta(cur: number | null, prev: number | null): string {
  const v = cur !== null ? String(cur) : '?'
  const d = delta(cur, prev)
  return `${v} Δ${d !== null ? d : '-'}`
}

function visLen(s: string): number {
  return s.replace(/\x1b\[[0-9;]*m/g, '').length
}

export function renderTable(rows: Cell[][]): string[] {
  const numCols = Math.max(...rows.map(r => r.length))

  const labelW: number[] = Array(numCols).fill(0)
  for (let col = 0; col < numCols; col++) {
    for (const row of rows) {
      if (col < row.length) {
        labelW[col] = Math.max(labelW[col], row[col].label.length)
      }
    }
  }

  const totalW: number[] = Array(numCols).fill(0)
  for (let col = 0; col < numCols; col++) {
    for (const row of rows) {
      if (col < row.length) {
        const w = labelW[col] + 1 + visLen(row[col].value)
        totalW[col] = Math.max(totalW[col], w)
      }
    }
  }

  return rows.map(row =>
    row
      .map((cell, col) => {
        const text = cell.label.padEnd(labelW[col]) + ' ' + cell.value
        const pad = totalW[col] - visLen(text)
        return text + (pad > 0 ? ' '.repeat(pad) : '')
      })
      .join('     '),
  )
}
