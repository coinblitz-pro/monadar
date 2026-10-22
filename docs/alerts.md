# Alerts and Telegram

Each tick every rule is evaluated against the data it needs. Warnings show inline (yellow/red
cells) and in the event history. Critical ones that persist `delay_ticks` go to Telegram.

| Alert | Level | Fires when |
|---|---|---|
| `RPC DOWN` | critical | `eth_blockNumber` fails |
| `STUCK` | critical | the block number has not changed for `stuck_sec` (5 s) |
| `NOT VOTING` | critical | our vote counter has not grown for 5 s while blocks advanced |
| `SYNCING` | critical | the node is state-syncing |
| `MISSED ROUND` | critical | one of our leader rounds missed or failed in the last 10 min ([rounds.md](rounds.md)) |
| `METRICS DOWN` | warning | no OTLP export for 15 s, or the metrics endpoint fails |
| `BFT LOG SILENT` | warning | the bft journal fed monadar and has been silent for 60 s |
| `LEDGER TAIL SILENT` | warning | the ledger-tail journal fed monadar and has been silent for 60 s |
| `LATE` | warning | we timed out on a round the network completed ([rounds.md](rounds.md)) |
| `NET DEGRADED` | warning | many network rounds end by TC; not our node ([rounds.md](rounds.md)) |
| `EXEC SLOW` | warning | execution latency p99 over the last 60 s exceeds `exec_ms_max` |
| `LAG` | warning | execution is more than `lag_max` blocks behind consensus |
| `LOW PEERS` | warning | peers below `peers_min` |
| `HIGH P99` | warning | broadcast p99 above `p99_max` |
| `SLOW RPC` | warning | RPC response above `rpc_max` |
| `TMO`, `TC` | warning | local timeouts / TCs grew; only without round data from the bft journal |

Thresholds are set by [flags or the config file](configuration.md#flags).

`NOT VOTING` fires when our vote counter has not grown for 5 s while blocks advanced. The
counter comes with the OTLP export, which can land a tick after the block, so a single flat
tick is not reported.

A rule whose data source is unavailable is **unknown**, not clear. When RPC is down, the
metrics endpoint is down, the execution journal goes quiet, or a metric disappears, the rules
that depend on it keep their state: nothing is falsely resolved, and nothing new fires on
missing data. The source outage is reported itself: `RPC DOWN` (critical), `METRICS DOWN`,
`BFT LOG SILENT` and `LEDGER TAIL SILENT` (a journal fed monadar and stopped: the service or
journalctl died, or journal access was lost). Rules with their own data keep working: `STUCK` still fires while
only the metrics endpoint is down. journalctl errors (missing permissions, for example) are
printed to the daemon's stderr.

## Telegram delivery

Alerts go through a delivery queue:

- A message leaves the queue only after Telegram answers `ok: true`. Network errors and 5xx are
  retried with backoff (1 s up to 60 s). 429 waits `retry_after`. Other 4xx (bad token, unknown
  chat) are dropped with a line in the daemon log.
- Every message carries the time of the event. A message that arrives over a minute late says
  so: "delivered 5m late". This happens when the host lost its uplink while the alert was queued.
- If an alert is still queued when its condition resolves, both go out as one message:
  "RPC DOWN · resolved", "Lasted 6s · 12:03:10 → 12:03:16".
- The queue is saved next to the event log (`telegram-outbox.json`), so a daemon restart during
  an outage loses nothing.
- Repeats within `cooldown_min` are not sent. The next message for that condition says how many
  were skipped. A skipped alert gets no "resolved" message either.
- Messages are Telegram Rich Messages (Bot API 10.1+): a heading, the reason, a table with the
  node's state, and a footer with the host name and event time. If Telegram rejects a rich
  message (for example, an older Bot API server behind `api_url`), monadar sends the same
  message as HTML and keeps using HTML until it restarts.
- `monadar telegram-test` sends one message of each kind (alert, alert resolved while queued,
  resolved, late alert, restart, `/status`, `/events`, `/help`) to the configured chat. It uses
  the running daemon's live data, so you can check how messages look.
- Each request uses its own connection with `SO_LINGER=0`. A timed-out request is discarded by
  the kernel, not retransmitted minutes later when the network returns.

A **node restart** is detected via a drop in `monad_total_uptime_us` (uptime is monotonic, so a
drop means the process restarted). On restart all node counters reset to 0, so the tick's
counter deltas are neutralized to avoid bogus values, the event is logged to the history, and a
one-off Telegram notification is sent.

Every event (warnings, resolves, restarts) is appended to a persistent **event log** so history
survives a daemon restart. On startup the daemon reloads the most recent events back into the
strip shown under the status line. The full event log is browsable in the viewer via **Ctrl+E**.

## Telegram bot

When `telegram.token`/`chat_id` are configured, the daemon also runs a **Telegram bot** (via
long polling: no public endpoint needed, works behind a firewall) that answers on demand:

| Command | Reply |
|---------|-------|
| `/status` | Active warnings; node state (block, execution, peers, broadcast p99, RPC, sync, votes, stake, uptime, version, key); our leader rounds for 1h and 24h; network rounds and TCs |
| `/events` | The recent events with date and time |
| `/help` | List of commands |

Only messages from the configured `chat_id` are answered. Telegram allows a single `getUpdates`
consumer per token, so don't point a webhook or a second daemon at the same bot token.
