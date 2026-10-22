export type Thresholds = {
  peers_min: number
  p99_max: number
  lag_max: number
  rpc_max: number
  tc_max: number
  exec_ms_max: number
}

export type ExecStats = { count: number; p50: number; p99: number; max: number }

// Leader-round counters (bft_log.ts). led/ok/missed/failed/notReceived/committed/txs
// are for our node; netRounds/netMissed are network-wide (rounds seen / rounds ended by TC).
export type RoundAgg = {
  led: number
  ok: number
  missed: number
  failed: number
  notReceived: number
  committed: number
  txs: number
  netRounds: number
  netMissed: number
}

export type RoundSummary = {
  h1: RoundAgg
  h24: RoundAgg
  missed10m: number
  lastMissed: { round: number; t: number } | null
  roundsPerSec: number
}

export type Config = {
  telegram?: { token: string; chat_id: string; delay_ticks?: number; cooldown_min?: number }
  thresholds: Thresholds
  log?: { file?: string; flush_sec?: number; max_size_mb?: number; keep?: number; events?: string }
  cpu?: string
  socket?: string
  // Built-in OTLP/gRPC receiver address ('' disables it → scrape METRICS_URL).
  otlp?: string
  // secp_key label to attach to OTLP metrics (otelcol used to add it via config).
  node_key?: string
}

export type Warning = { level: 'warn' | 'crit'; key: string; short: string; detail: string; notify?: string }
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
  // Execution latency (proposal accepted → block executed) over the last 60s,
  // parsed from monad-execution's journal. null when no recent log lines.
  exec: ExecStats | null
  // Our leader rounds and network round stats from the bft journal (DEBUG lines).
  rounds: RoundSummary | null
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
  execP99: number | null
  led24h: number | null
  missed24h: number | null
  peers: number | null
  p99: number | null
  rpcMs: number
  syncing: number | null
  syncPct: string | null
  uptime: string
  ver: string
}
