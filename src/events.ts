import { appendFileSync, mkdirSync, readFileSync } from 'fs'
import { dirname } from 'path'
import { eventsFile } from './config'
import type { HistoryEntry } from './types'

// Stored events carry a local `YYYY-MM-DD HH:MM:SS` timestamp so display is
// consistent with the dashboard (which uses local time), and `at.slice(5,10)`
// / `at.slice(11,19)` cleanly yield the MM-DD and HH:MM:SS parts.
export type StoredEvent = { at: string; level: HistoryEntry['level']; text: string }

let ensured = false
function ensureDir() {
  if (ensured || !eventsFile) return
  try { mkdirSync(dirname(eventsFile), { recursive: true }) } catch {}
  ensured = true
}

function stamp(): string {
  const d = new Date()
  const p = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} `
    + `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

// Append one event as `timestamp<TAB>level<TAB>text`. Called only on warning
// state changes / restarts (low-frequency), so a sync append is fine and keeps
// the file consistent without a flush timer.
export function persistEvent(e: HistoryEntry) {
  if (!eventsFile) return
  ensureDir()
  try { appendFileSync(eventsFile, `${stamp()}\t${e.level}\t${e.text}\n`) } catch {}
}

// All persisted events, oldest first. Read on demand by the events overlay.
export function loadEvents(): StoredEvent[] {
  if (!eventsFile) return []
  let raw: string
  try { raw = readFileSync(eventsFile, 'utf8') } catch { return [] }
  const out: StoredEvent[] = []
  for (const line of raw.split('\n')) {
    if (!line) continue
    const tab1 = line.indexOf('\t')
    const tab2 = line.indexOf('\t', tab1 + 1)
    if (tab1 === -1 || tab2 === -1) continue
    out.push({
      at: line.slice(0, tab1),
      level: line.slice(tab1 + 1, tab2) as HistoryEntry['level'],
      text: line.slice(tab2 + 1),
    })
  }
  return out
}
