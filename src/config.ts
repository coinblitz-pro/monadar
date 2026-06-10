import { readFileSync } from 'fs'
import { dirname, resolve } from 'path'
import type { Config } from './types'

export const RPC_URL = 'http://localhost:8080'
export const METRICS_URL = 'http://localhost:8889/metrics'

const args = process.argv.slice(2)
export const verbose = args.includes('--verbose') || args.includes('-v')
export const bell = args.includes('--bell')
export const shouldInit = args.includes('--init')

const logIdx = args.findIndex(a => a === '--log')
const logFileArg = logIdx !== -1 ? (args[logIdx + 1] ?? 'monadar.log') : null

const configIdx = args.findIndex(a => a === '--config')
const configPath = configIdx !== -1 ? (args[configIdx + 1] ?? null) : null

const socketIdx = args.findIndex(a => a === '--socket')
const socketArg = socketIdx !== -1 ? (args[socketIdx + 1] ?? null) : null

const usedArgIndices = new Set<number>()
for (const flag of [ '--log', '--config', '--socket', '--peers-min', '--p99-max', '--lag-max', '--rpc-max', '--tc-max', '--cpu' ]) {
  const i = args.findIndex(a => a === flag)
  if (i !== -1) {
    usedArgIndices.add(i)
    usedArgIndices.add(i + 1)
  }
}

export const interval = parseInt(
  args.find((a, i) => /^\d+$/.test(a) && !usedArgIndices.has(i)) ?? '1', 10,
) * 1000

const DEFAULT_THRESHOLDS = { peers_min: 50, p99_max: 500, lag_max: 5, rpc_max: 200, tc_max: 0 }

function loadConfig(): Config {
  const p = configPath ?? resolve(dirname(process.argv[1]), 'monadar.config.json')
  try {
    const raw = readFileSync(p, 'utf-8')
    const json = JSON.parse(raw)
    return {
      telegram: json.telegram,
      thresholds: { ...DEFAULT_THRESHOLDS, ...json.thresholds },
      log: json.log,
      cpu: json.cpu != null ? String(json.cpu) : undefined,
      socket: json.socket || undefined,
    }
  } catch {
    return { thresholds: { ...DEFAULT_THRESHOLDS } }
  }
}

export const config = loadConfig()

export const socket = socketArg || config.socket || null

const cpuIdx = args.findIndex(a => a === '--cpu')
export const cpu = cpuIdx !== -1 ? args[cpuIdx + 1] : config.cpu ?? null

const argVal = (name: string): number | null => {
  const i = args.findIndex(a => a === name)
  return i !== -1 && args[i + 1] ? parseInt(args[i + 1], 10) : null
}

export const thresholds = {
  peers_min: argVal('--peers-min') ?? config.thresholds.peers_min,
  p99_max: argVal('--p99-max') ?? config.thresholds.p99_max,
  lag_max: argVal('--lag-max') ?? config.thresholds.lag_max,
  rpc_max: argVal('--rpc-max') ?? config.thresholds.rpc_max,
  tc_max: argVal('--tc-max') ?? config.thresholds.tc_max,
}

export const logFile = logFileArg || config.log?.file || null
export const logFlushMs = (config.log?.flush_sec ?? 60) * 1000
// Rotate the log when it reaches this size, keeping `logKeep` numbered backups
// (file.1 … file.N). max_size_mb <= 0 disables rotation (unbounded growth).
export const logMaxBytes = Math.round((config.log?.max_size_mb ?? 50) * 1024 * 1024)
export const logKeep = config.log?.keep ?? 5
