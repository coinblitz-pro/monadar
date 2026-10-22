# How monadar works

`monadar` runs as a **background daemon** (under systemd/launchd) that does the polling,
keeps the deltas/history, and sends Telegram alerts continuously, and a thin **attach viewer**
that connects to the daemon over a unix socket and renders the dashboard. The viewer can be
opened and closed freely without interrupting monitoring; multiple viewers can attach at once.

```
 daemon ──(RPC, OTLP metrics, journals → deltas, warnings, Telegram)──► Frame
   │  broadcast Frame as JSON lines over a unix socket
   ▼
 viewer ── connect, render dashboard, local Ctrl+O / Ctrl+H overlays
```

The full tick runs once a second: the node exports metrics over OTLP once a second
(`--record-metrics-interval-seconds 1`), and alerts and the log follow the tick. Between ticks,
while a viewer is attached, the daemon polls local RPC every 100 ms and sends a frame whenever
the block or the executed block (from the `monad-execution` journal) moves, so these two and the
execution lag update with the chain, about every 0.3-0.4 s. RPC shows a new block about 0.2 s
before journald delivers the bft line about it, so RPC is the faster source. Without a live
execution journal the dashboard updates once a second.

A standalone `--foreground` mode (poll + render in one process, no daemon) is kept for quick
checks and debugging.

## Data sources

| Source | What monadar takes from it |
|---|---|
| JSON-RPC `eth_blockNumber` (`localhost:8080`) | The block number: the newest proposed block, ahead of execution. Polled every tick, and every 100 ms while a viewer is attached. |
| OTLP metrics (`--otel-endpoint` → `127.0.0.1:4317`) | Counters and gauges: votes, commits, timeouts, TCs, peers, sessions, stake, sync, uptime, version, txpool, broadcast p99. See [metrics.md](metrics.md). Falls back to scraping `localhost:8889/metrics` when the port is taken. |
| `monad-execution` journal | `__exec_block` lines: the executed block and execution latency. |
| `monad-bft` journal (DEBUG) | Leader rounds, proposals, TCs, local timeouts, block sizes, forwarded transactions, our own key. See [rounds.md](rounds.md). |

Execution latency comes from `__exec_block` lines in the `monad-execution` journal: each line
has the time execution finished and `ts`, when BFT accepted the proposal. The daemon runs
`journalctl -f`, so it needs root or the `systemd-journal` group. If there are no recent lines,
monadar falls back to the `monad_execution_ledger_block_num` metric. That metric lags by the
OTLP export interval.

The daemon runs `journalctl -f -n 0 -o cat -u <unit>` for both journals and restarts it if it
exits. journalctl's own errors (no permission, unknown unit) go to the daemon's stderr, each
distinct message once.

## Our key

Our key is `node_key` when set. Otherwise monadar learns it from the bft journal: only our own node logs `emitting create proposal command to txpool` with its `node_id`. Until then it uses the `secp_key` metric label. If `node_key` disagrees with the journal, the daemon says so in its log.
