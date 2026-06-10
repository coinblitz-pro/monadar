import { readFileSync } from 'fs'
import { dirname, resolve } from 'path'
import { parseArgs } from './args'
import type { Config, Thresholds } from './types'

export const RPC_URL = 'http://localhost:8080'
export const METRICS_URL = 'http://localhost:8889/metrics'

export const cliArgs = parseArgs(process.argv.slice(2))
export const verbose = cliArgs.verbose
export const bell = cliArgs.bell
export const shouldInit = cliArgs.shouldInit
export const interval = cliArgs.intervalSec * 1000

export const DEFAULT_THRESHOLDS: Thresholds = { peers_min: 50, p99_max: 500, lag_max: 5, rpc_max: 200, tc_max: 0 }

export function loadConfig(configPath: string | null, entrypoint: string): Config {
  const p = configPath ?? resolve(dirname(entrypoint), 'monadar.config.json')
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

export const config = loadConfig(cliArgs.configPath, process.argv[1])
export const socket = cliArgs.socket || config.socket || null
export const cpu = cliArgs.cpu ?? config.cpu ?? null

export const thresholds = {
  peers_min: cliArgs.thresholdOverrides.peers_min ?? config.thresholds.peers_min,
  p99_max: cliArgs.thresholdOverrides.p99_max ?? config.thresholds.p99_max,
  lag_max: cliArgs.thresholdOverrides.lag_max ?? config.thresholds.lag_max,
  rpc_max: cliArgs.thresholdOverrides.rpc_max ?? config.thresholds.rpc_max,
  tc_max: cliArgs.thresholdOverrides.tc_max ?? config.thresholds.tc_max,
}

export const logFile = cliArgs.logFile || config.log?.file || null
// Persistent event log (warning state changes, restarts) — survives daemon
// restarts. Defaults next to the main log file; disabled if neither is set.
export const eventsFile = config.log?.events
  || (logFile ? `${dirname(logFile)}/events.log` : null)
export const logFlushMs = (config.log?.flush_sec ?? 60) * 1000
// Rotate the log when it reaches this size, keeping `logKeep` numbered backups
// (file.1 … file.N). max_size_mb <= 0 disables rotation (unbounded growth).
export const logMaxBytes = Math.round((config.log?.max_size_mb ?? 50) * 1024 * 1024)
export const logKeep = config.log?.keep ?? 5
