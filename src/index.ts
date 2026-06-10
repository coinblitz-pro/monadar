import { execFileSync } from 'child_process'
import { existsSync, writeFileSync } from 'fs'
import { dirname, resolve } from 'path'
import { seedHistory } from './alerts'
import { attach } from './client'
import { createCollector } from './collect'
import { bell, cpu, interval, shouldInit, socket } from './config'
import { runDaemon } from './daemon'
import { install, status, uninstall } from './install'
import { flushLog, initLogTimer, logLine } from './log'
import { setSocketPath } from './protocol'
import { applyFrame, paint } from './render'
import { setupInput } from './ui'

const args = process.argv.slice(2)

setSocketPath(socket)

if (shouldInit) {
  const p = resolve(dirname(process.argv[1]), 'monadar.config.json')
  if (existsSync(p)) {
    console.log(`Config already exists: ${p}`)
  } else {
    const template = JSON.stringify({
      telegram: { token: '', chat_id: '', delay_ticks: 3, cooldown_min: 5 },
      thresholds: { peers_min: 50, p99_max: 500, lag_max: 5, rpc_max: 200, tc_max: 0 },
      log: { file: '', flush_sec: 60, max_size_mb: 50, keep: 5, events: '' },
      socket: '',
      cpu: '',
    }, null, 2)
    writeFileSync(p, template + '\n')
    console.log(`Created ${p}`)
  }
  process.exit(0)
}

function applyAffinity() {
  if (!cpu) return
  try {
    execFileSync('taskset', [ '-cp', cpu, String(process.pid) ])
  } catch {
    console.error(`Failed to set CPU affinity to ${cpu}`)
    process.exit(1)
  }
}

// Standalone mode: poll + render in one process, no daemon/socket (old behavior).
async function runForeground() {
  seedHistory()

  async function shutdown() {
    await flushLog()
    if (process.stdin.isTTY) process.stdin.setRawMode(false)
    process.stdout.write('\x1b[?25h\n')
    process.exit()
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)

  initLogTimer()
  setupInput(paint, shutdown)
  process.stdout.write('\x1b[2J\x1b[H\x1b[?25l')

  const collector = createCollector()
  while (true) {
    const frame = await collector.tick()
    if (frame.logLine) logLine(frame.logLine)
    if (bell && frame.newCrit) process.stdout.write('\x07')
    applyFrame(frame)
    paint()
    await new Promise((r) => setTimeout(r, interval))
  }
}

if (args.includes('install')) {
  install({ print: args.includes('--print') })
} else if (args.includes('uninstall')) {
  uninstall()
} else if (args.includes('status')) {
  status()
} else {
  applyAffinity()
  if (args.includes('daemon')) {
    runDaemon()
  } else if (args.includes('--foreground')) {
    runForeground()
  } else {
    attach()
  }
}
