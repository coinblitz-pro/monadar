export type Thresholds = {
  peers_min: number
  p99_max: number
  lag_max: number
  rpc_max: number
  tc_max: number
}

export type Config = {
  telegram?: { token: string; chat_id: string; delay_ticks?: number; cooldown_min?: number }
  thresholds: Thresholds
  log?: { file?: string; flush_sec?: number; max_size_mb?: number; keep?: number; events?: string }
  cpu?: string
  socket?: string
}

export type Warning = { level: 'warn' | 'crit'; key: string; short: string; detail: string }
export type HistoryEntry = { time: string; level: 'warn' | 'crit' | 'error' | 'resolved'; text: string }
export type Cell = { label: string; value: string }

export type CounterCell = { value: number | null; delta: number | null }

export type Frame = {
  ts: string
  error?: 'rpc' | 'metrics'
  block: number
  blockDelta: number | null
  bps: string
  ver: string
  uptime: string
  syncing: number | null
  syncPct: string | null
  peers: number | null
  sessions: number | null
  commits: CounterCell
  votes: CounterCell
  timeouts: CounterCell
  tc: CounterCell
  execLag: number
  lagEvtDelta: number | null
  execBlock: number | null
  tx: CounterCell
  txpool: number | null
  fwd: CounterCell
  p99: number | null
  rpcMs: number
  key: string
  network: string
  stakePct: string
  proposals: CounterCell
  leaderPct: string
  votedPct: number | null
  buildVersion: string
  status: 'crit' | 'warn' | 'ok'
  newCrit: boolean
  warnings: Warning[]
  history: HistoryEntry[]
  logLine: string
}

export type ServerMsg =
  | { type: 'hello'; backlog: string[]; frame: Frame | null }
  | { type: 'snapshot'; frame: Frame }

export type Snapshot = {
  block: number
  blockDelta: number | null
  bps: string
  lag: number
  peers: number | null
  p99: number | null
  rpcMs: number
  syncing: number | null
  syncPct: string | null
  uptime: string
  ver: string
}
