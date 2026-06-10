function metricPrefixMatches(line: string, name: string): boolean {
  return line.startsWith(name + '{') || line.startsWith(name + ' ')
}

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function metricValue(text: string, name: string): number | null {
  let result: number | null = null
  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim()
    if (!line || line.startsWith('#') || !metricPrefixMatches(line, name)) continue
    const val = line.split(/\s+/)[1]
    if (val !== undefined) {
      const n = Number(val)
      if (Number.isFinite(n)) result = n
    }
  }
  return result
}

export function metricLabel(text: string, metric: string, label: string): string {
  let result = '?'
  const labelPattern = escapeRegExp(label)
  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim()
    if (metricPrefixMatches(line, metric)) {
      const m = line.match(new RegExp(`${labelPattern}="((?:\\\\.|[^"\\\\])*)"`))
      if (m?.[1]) result = m[1]
    }
  }
  return result
}
