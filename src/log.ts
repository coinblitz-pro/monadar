import { mkdirSync } from 'fs'
import { appendFile, rename, stat, unlink } from 'fs/promises'
import { dirname } from 'path'
import { logFile, logFlushMs, logKeep, logMaxBytes } from './config'

if (logFile) {
  try { mkdirSync(dirname(logFile), { recursive: true }) } catch {}
}

let logBuffer: string[] = []
let flushing = false

// In-memory ring buffer for the Ctrl+O log view (filled regardless of --log)
const recent: string[] = []
const MAX_RECENT = 1000

// Size-based rotation: when the log reaches logMaxBytes, shift backups
// (file.N-1 → file.N, …, file.1 → file.2) and move the current file to file.1,
// so the live file starts fresh. Keeps at most logKeep numbered backups.
async function rotateIfNeeded() {
  if (!logFile || logMaxBytes <= 0) return
  let size: number
  try { size = (await stat(logFile)).size } catch { return }
  if (size < logMaxBytes) return
  if (logKeep < 1) {
    try { await unlink(logFile) } catch {}
    return
  }
  for (let i = logKeep - 1; i >= 1; i--) {
    try { await rename(`${logFile}.${i}`, `${logFile}.${i + 1}`) } catch {}
  }
  try { await rename(logFile, `${logFile}.1`) } catch {}
}

export async function flushLog() {
  if (!logFile || logBuffer.length === 0 || flushing) return
  flushing = true
  const chunk = logBuffer.join('')
  logBuffer = []
  try {
    await appendFile(logFile, chunk)
    await rotateIfNeeded()
  } catch {}
  flushing = false
}

export function logLine(line: string) {
  recent.push(line)
  if (recent.length > MAX_RECENT) recent.shift()
  if (logFile) logBuffer.push(line + '\n')
}

export function getRecent(n: number): string[] {
  return recent.slice(-n)
}

export function recentCount(): number {
  return recent.length
}

export function initLogTimer() {
  if (logFile) setInterval(flushLog, logFlushMs)
}
