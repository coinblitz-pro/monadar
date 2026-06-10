import { loadEvents, persistEvent } from './events'
import type { HistoryEntry } from './types'

const MAX_HISTORY = 5
const warningHistory: HistoryEntry[] = []

export type HistoryStore = {
  push(entry: HistoryEntry): void
}

export function pushHistory(entry: HistoryEntry) {
  warningHistory.push(entry)
  if (warningHistory.length > MAX_HISTORY) warningHistory.shift()
  persistEvent(entry)
}

export function seedHistory() {
  warningHistory.length = 0
  for (const e of loadEvents().slice(-MAX_HISTORY)) {
    warningHistory.push({ time: e.at.slice(11, 19), level: e.level, text: e.text })
  }
}

export function getHistory(): HistoryEntry[] {
  return warningHistory.slice()
}
