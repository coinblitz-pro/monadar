export function metricValue(text: string, name: string): number | null {
  let result: number | null = null
  for (const line of text.split('\n')) {
    if (line.startsWith(name + '{')) {
      const val = line.split(/\s+/)[1]
      if (val !== undefined) result = Number(val)
    }
  }
  return result
}

export function metricLabel(text: string, metric: string, label: string): string {
  let result = '?'
  for (const line of text.split('\n')) {
    if (line.startsWith(metric + '{')) {
      const m = line.match(new RegExp(`${label}="([^"]*)"`))
      if (m?.[1]) result = m[1]
    }
  }
  return result
}
