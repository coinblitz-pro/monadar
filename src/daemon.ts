import { chmodSync, existsSync, unlinkSync } from 'fs'
import { createServer, type Socket } from 'net'
import { createCollector } from './collect'
import { interval } from './config'
import { seedHistory } from './event_history'
import { flushLog, getRecent, initLogTimer, logLine } from './log'
import { encodeMsg } from './protocol'
import { socketPath } from './socket'
import type { Frame } from './types'

const BACKLOG_LINES = 200

// Headless collector: polls on the interval, broadcasts each Frame to connected
// viewers over a unix socket, writes the log file, and sends Telegram alerts.
export async function runDaemon() {
  seedHistory()
  const path = socketPath()
  if (existsSync(path)) {
    // Stale socket from a previous run, or another daemon. Try to remove it;
    // if a live daemon is already bound, listen() below will fail loudly.
    try { unlinkSync(path) } catch {}
  }

  const clients = new Set<Socket>()
  let lastFrame: Frame | null = null

  const server = createServer((sock) => {
    sock.setNoDelay(true)
    clients.add(sock)
    sock.on('error', () => clients.delete(sock))
    sock.on('close', () => clients.delete(sock))
    // Greet with recent log backlog (for the Ctrl+O view) + the latest frame.
    sock.write(encodeMsg({ type: 'hello', backlog: getRecent(BACKLOG_LINES), frame: lastFrame }))
  })

  server.on('error', (err) => {
    console.error(`Socket error on ${path}: ${(err as Error).message}`)
    process.exit(1)
  })

  await new Promise<void>((res) => server.listen(path, res))
  try { chmodSync(path, 0o666) } catch {}
  console.error(`monadar daemon listening on ${path}`)

  initLogTimer()

  let stopping = false
  async function shutdown() {
    if (stopping) return
    stopping = true
    for (const s of clients) s.destroy()
    server.close()
    try { unlinkSync(path) } catch {}
    await flushLog()
    process.exit()
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)

  const collector = createCollector()
  while (!stopping) {
    const frame = await collector.tick()
    lastFrame = frame
    if (frame.logLine) logLine(frame.logLine)
    const msg = encodeMsg({ type: 'snapshot', frame })
    for (const s of clients) {
      try { s.write(msg) } catch {}
    }
    await new Promise((r) => setTimeout(r, interval))
  }
}
