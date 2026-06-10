export type Config = {
  telegram?: { token: string; chat_id: string; delay_ticks?: number; cooldown_min?: number };
  thresholds: { peers_min: number; p99_max: number; lag_max: number; rpc_max: number; tc_max: number };
  log?: { file?: string; flush_sec?: number; max_size_mb?: number; keep?: number; events?: string };
  cpu?: string;
  socket?: string;
};

export type { CounterCell, Frame, ServerMsg } from './protocol';

export type Warning = { level: 'warn' | 'crit'; key: string; short: string; detail: string };
export type HistoryEntry = { time: string; level: 'warn' | 'crit' | 'error' | 'resolved'; text: string };
export type Cell = { label: string; value: string };

export type Snapshot = {
  block: number;
  blockDelta: number | null;
  bps: string;
  lag: number;
  peers: number | null;
  p99: number | null;
  rpcMs: number;
  syncing: number | null;
  syncPct: string | null;
  uptime: string;
  ver: string;
};
