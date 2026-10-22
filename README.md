<p align="center">
  <img src="assets/logo.png" alt="monadar" width="160">
</p>

# monadar

CLI monitoring tool for a [Monad](https://monad.xyz) node. Polls the node's JSON-RPC
(`eth_blockNumber`) and Prometheus metrics endpoint, then renders a live status dashboard
with colored alerts, an event history, and optional Telegram notifications.

```
monadar  10:20:35

block:   80336279 Δ2 2.47/s     ver:   0.14.4         up:     4h53m     sync: no          peers:  222       sessions: 268
commits: 44095 Δ2               votes: 43776 Δ3       tmo:    12 Δ0     tc:   11 Δ0       lag:    0         lag_evt:  Δ0
exec:    80336277 (-2)          tx:    560371 Δ33     txpool: 4         fwd:  9613 Δ2     p99:    113ms     rpc:      0ms
key:     0xdead..beef           net:   mainnet        stake:  0.36%     prop: 144 Δ0      leader: 0.33%     voted:    99.3%

● OK
```

## Architecture

`monadar` runs as a **background daemon** (under systemd/launchd) that does the polling,
keeps the deltas/history, and sends Telegram alerts continuously — and a thin **attach viewer**
that connects to the daemon over a unix socket and renders the dashboard. The viewer can be
opened and closed freely without interrupting monitoring; multiple viewers can attach at once.

```
 daemon ──(poll RPC+metrics, deltas, warnings, Telegram)──► Frame
   │  broadcast Frame as JSON lines over a unix socket
   ▼
 viewer ── connect, render dashboard, local Ctrl+O / Ctrl+H overlays
```

A standalone `--foreground` mode (poll + render in one process, no daemon) is kept for quick
checks and debugging.

## Requirements

- Rust 1.88+ to build (one binary, no runtime needed on the node)
- A reachable Monad node: RPC at `http://localhost:8080`, metrics at `http://localhost:8889/metrics`

## Build

```sh
cargo build --release   # → target/release/monadar
cargo test              # unit tests for parsing, frame math, metrics, OTLP, service generation
cargo clippy --all-targets
```

`src/fixture_replay.rs` replays bft/exec journals and raw OTLP exports captured on a mainnet
validator and checks the results against the TS reference numbers. The fixtures (~100 MB) are
not in the repo; point the test at an unpacked fixture directory:

```sh
MONADAR_FIXTURES=/path/to/2026-10-22 cargo test --release fixture -- --nocapture
```

For the node (x86_64 Linux) build a static musl binary. The script runs the build in an
x86_64 Alpine container, so it works from macOS too (needs Docker). It uses the baseline
x86-64 CPU target, not `target-cpu=native`, so the binary runs on any x86_64 host:

```sh
./scripts/build-musl.sh  # → target/x86_64-unknown-linux-musl/release/monadar
```

The binary is self-contained: copy it to the node (e.g. `/usr/local/bin/monadar`).

### Releases and verification

Pushing a tag `v*` runs `.github/workflows/release.yml`. The workflow tests, builds with the
same `scripts/build-musl.sh`, and publishes `monadar-<tag>-x86_64-linux-musl` with `SHA256SUMS`
as a GitHub release.

The build is reproducible. The toolchain is a single container image pinned by digest (rustc,
musl, linker), crates are pinned by `Cargo.lock`, and paths inside the container are fixed. So
anyone can check that a release binary was built from the tagged source:

```sh
sha256sum -c SHA256SUMS                         # the download is intact
git checkout v1.0.0 && ./scripts/build-musl.sh  # prints the same sha256 as SHA256SUMS
```

`monadar --version` prints the build: `git describe` of the source, which is the tag for a
release. The viewer uses it to tell when the daemon runs a different build. When the repository
is public, the workflow also attaches a signed build provenance attestation, and
`gh attestation verify <file> -R coinblitz-pro/monadar` checks it.

The examples below use `monadar` to mean that binary.

## Quick start

```sh
monadar install            # write a system service (run with sudo)
sudo systemctl enable --now monadar   # Linux
monadar                    # attach a viewer to the running daemon
```

Without a daemon, a bare `monadar` prints how to start one and exits. To run without a
service at all, use `monadar --foreground`.

## Commands

| Command | Description |
|---------|-------------|
| `monadar` | Attach a viewer to the running daemon (default) |
| `monadar --foreground` | Standalone poll + render in one process (no daemon) |
| `monadar daemon` | Run the headless collector + socket server (used by the service) |
| `monadar install [--print]` | Generate the systemd unit / launchd plist (`--print` writes it to stdout instead of installing) |
| `monadar uninstall` | Remove the service unit |
| `monadar status` | Probe the socket and report whether the daemon is running |
| `monadar telegram-test` | Send sample alerts, `/status` and `/events` to the configured Telegram chat |
| `monadar --init` | Write a `monadar.config.json` template next to the binary |
| `monadar --version` | Print the build version (`git describe` of the source) |

### Service setup

`install` is **system-level** and writes:

- Linux: `/etc/systemd/system/monadar.service` (`Restart=always`, `WantedBy=multi-user.target`)
- macOS: `/Library/LaunchDaemons/com.monadar.plist` (`RunAtLoad`, `KeepAlive`)

```sh
# Linux
sudo monadar install          # writes unit + runs daemon-reload
sudo systemctl enable --now monadar

# macOS
sudo monadar install
sudo launchctl load -w /Library/LaunchDaemons/com.monadar.plist
```

Flags passed to `install` (e.g. `--config`, `--socket`, `--cpu`, `--log`) are baked into the
unit's `ExecStart`, so the daemon runs with the same configuration.

## Flags

| Flag | Description | Default |
|------|-------------|---------|
| `<seconds>` | Polling interval (positional integer) | `1` |
| `--config <path>` | Path to `monadar.config.json` | next to the binary |
| `--socket <path>` | Unix socket path | `/run/monadar.sock`, else `$XDG_RUNTIME_DIR`/`/tmp` |
| `--log [file]` | Write the log file (defaults to `monadar.log` if no path given) | off |
| `--cpu <list>` | Pin the daemon to CPUs via `taskset` (e.g. `2,3`) | off |
| `--bell` | Terminal bell on a new critical alert | off |
| `--verbose`, `-v` | Open the parameter help panel on start | off |
| `--peers-min <n>` | Warn when peers drop below this | `50` |
| `--p99-max <ms>` | Warn when broadcast p99 exceeds this | `500` |
| `--lag-max <n>` | Warn when execution lag exceeds this many blocks | `5` |
| `--rpc-max <ms>` | Warn when RPC response time exceeds this | `200` |
| `--tc-max <n>` | Warn when timeout-certificate delta exceeds this | `0` |
| `--exec-ms-max <ms>` | Warn when execution latency p99 (last 60s) exceeds this | `250` |
| `--stuck-sec <s>` | Critical STUCK when the block number has not changed for this long (one timed-out round alone pauses blocks for ~1.5s) | `5` |
| `--net-tc-pct-max <n>` | `NET DEGRADED` when more than n% of network rounds in the last 10 min ended by TC (normal is ~0.7%) | `3` |
| `--net-tc-run-max <n>` | `NET DEGRADED` when more than n TC rounds came back to back in the last 10 min (the whole chain stands still) | `1` |

Execution latency comes from `__exec_block` lines in the `monad-execution` journal: each line has the time execution finished and `ts`, when BFT accepted the proposal. The daemon runs `journalctl -f`, so it needs root or the `systemd-journal` group. If there are no recent lines, monadar falls back to the `monad_execution_ledger_block_num` metric. That metric lags by the OTLP export interval.

Leader rounds come from monad-bft's DEBUG journal lines `leader for round`, `proposal message`, `advancing round` and `committed block`. monadar follows this journal with `journalctl -f` too. A round is classified 10 rounds after it happened:

| Proposal from the leader | Timeout certificate for the round | Result |
|---|---|---|
| yes | no | `ok` |
| no | yes | `missed` (the leader's block never came) |
| yes | yes | `failed` (proposed but did not get enough votes) |
| no | no | `unseen` (the network moved on, but this node never saw the proposal) |

The dashboard row shows our rounds over 24h (with 1h next to it): `led24h`, `missed` (missed + failed), `unseen`, committed `blocks` and their `txs`. It also shows `net_tc`, the share of all network rounds that ended by TC, with rounds/s. A critical `MISSED ROUND` alert, sent to Telegram after `delay_ticks`, fires while any of our rounds missed or failed in the last 10 minutes. A `LATE` warning fires when our node logged a `local timeout` in the last 10 minutes for a round the network still completed with a QC (no TC). That points to our own network or CPU, not to the leader. While round data is available, the metric-based `TMO`/`TC` warnings are off. Those counters also grow on every other leader's miss, so they mostly add noise. The `tmo` cell is highlighted only for `LATE`. Block size: each of our proposals is compared with the median tx count of the 50 network blocks right before it. Network load changes during the day, and the median ignores outlier blocks. The `txs` cell shows the 24h ratio as `N% net` (100% = same size as neighbours, yellow below 80%). `/status` shows the ratio for 1h and 24h, plus how many transactions other nodes forwarded to our txpool. RPC has no mempool methods, so this forwarded stream is our view of the mempool. Our key is `node_key` when set. Otherwise monadar learns it from the bft journal: only our own node logs `emitting create proposal command to txpool` with its `node_id`. Until then it uses the `secp_key` metric label. If `node_key` disagrees with the journal, the daemon says so in its log. Counters are kept in memory and reset when monadar restarts. If bft logs at INFO only, the row is not shown.

## Configuration

`monadar --init` writes a template `monadar.config.json`:

```json
{
  "telegram": { "token": "", "chat_id": "", "delay_ticks": 3, "cooldown_min": 5, "api_url": "" },
  "thresholds": { "peers_min": 50, "p99_max": 500, "lag_max": 5, "rpc_max": 200, "tc_max": 0, "exec_ms_max": 250, "stuck_sec": 5, "net_tc_pct_max": 3, "net_tc_run_max": 1 },
  "log": { "file": "", "flush_sec": 60, "max_size_mb": 50, "keep": 5, "events": "" },
  "socket": "",
  "cpu": "",
  "otlp": "127.0.0.1:4317",
  "node_key": "",
  "heartbeat_url": "",
  "heartbeat_sec": 60
}
```

- **telegram** — bot `token` and `chat_id` for alerts. `delay_ticks`: a critical alert must
  persist this many ticks before notifying (and a recovery as many ticks before "resolved" is
  sent); `cooldown_min`: minimum minutes between two alerts for the same condition. `rich`: send
  Telegram Rich Messages (headings, tables; default `true`), `false` for classic HTML messages.
  `api_url`: Bot API base, `https://api.telegram.org` by default (set it for a local Bot API
  server or proxy).
- **thresholds** — same values as the threshold flags above; CLI flags override the config.
- **log** — `file` path, flush interval `flush_sec`, rotation (see below), and `events` — the
  path of the persistent event log (defaults to `events.log` next to `file`).
- **socket** — override the unix socket path.
- **cpu** — `taskset` CPU list for the daemon.
- **otlp** — address of the built-in OTLP/gRPC metrics receiver (default `127.0.0.1:4317`, the
  same as the nodes' `--otel-endpoint`). monad-bft and monad-rpc push metrics to it directly, so
  no OpenTelemetry Collector is needed. If the port is taken (an otelcol is still running),
  monadar scrapes `http://localhost:8889/metrics` instead. The choice is made at startup, so
  restart monadar after removing otelcol. Set `""` to always scrape.
- **node_key** — the node's public secp256k1 key, reported as the `secp_key` label (otelcol used
  to add it through its `resource` processor). Optional: monadar learns it from the bft journal.
- **heartbeat_url** — dead man's switch. The daemon GETs this URL every `heartbeat_sec` (default
  60) seconds, e.g. a [healthchecks.io](https://healthchecks.io) check or an Uptime Kuma push
  monitor. That service alerts when the pings stop. This is the only way to hear about the host,
  its network or monadar itself going down: monadar cannot report its own outage.

## Logging & rotation

With `--log` (or `log.file` set), every tick appends one `key=val` line to the log file. The
file is rotated by size:

- `log.max_size_mb` (default `50`) — rotate when the file reaches this size. `<= 0` disables rotation.
- `log.keep` (default `5`) — number of numbered backups to keep (`monadar.log.1` … `monadar.log.N`);
  on rotation the live file becomes `monadar.log.1`, older backups shift up, and the oldest is dropped.

The viewer also keeps the most recent lines in memory for the **Ctrl+O** log view, regardless
of whether a log file is configured.

## Alerts & events

Each tick every rule is evaluated against the data it needs: low peers, high p99/RPC, execution
lag and latency, our missed and late rounds, network TC rate, plus critical conditions
(state-syncing, block not advancing, not voting, RPC down). Warnings show inline (yellow/red
cells) and in the event history. Critical ones that persist `delay_ticks` go to Telegram.

A rule whose data source is unavailable is **unknown**, not clear. When RPC is down, the
metrics endpoint is down, the execution journal goes quiet, or a metric disappears, the rules
that depend on it keep their state: nothing is falsely resolved, and nothing new fires on
missing data. The source outage is reported itself: `RPC DOWN` (critical), `METRICS DOWN`, and
`BFT LOG SILENT` (the bft journal fed monadar and stopped: journalctl died, no journal access, or
bft no longer logs at DEBUG). Rules with their own data keep working: `STUCK` still fires while
only the metrics endpoint is down. journalctl errors (missing permissions, for example) are
printed to the daemon's stderr.

### Telegram delivery

Alerts go through a delivery queue:

- A message leaves the queue only after Telegram answers `ok: true`. Network errors and 5xx are
  retried with backoff (1 s up to 60 s). 429 waits `retry_after`. Other 4xx (bad token, unknown
  chat) are dropped with a line in the daemon log.
- Every message carries the time of the event. A message that arrives over a minute late says
  so: "delivered 5m late". This happens when the host lost its uplink while the alert was queued.
- If an alert is still queued when its condition resolves, both go out as one message:
  "RPC DOWN — resolved after 6s, 12:03:10 → 12:03:16".
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
survives a daemon restart — on startup the daemon reloads the most recent events back into the
strip shown under the status line. The full event log is browsable in the viewer via **Ctrl+E**.

### Telegram bot

When `telegram.token`/`chat_id` are configured, the daemon also runs a **Telegram bot** (via
long polling — no public endpoint needed, works behind a firewall) that answers on demand:

| Command | Reply |
|---------|-------|
| `/status` | Current block, lag, peers, p99, RPC, sync, uptime, version + active warnings |
| `/events` | The recent-events list |
| `/help` | List of commands |

Only messages from the configured `chat_id` are answered. Telegram allows a single `getUpdates`
consumer per token, so don't point a webhook or a second daemon at the same bot token.

## Dashboard

Four rows; cells turn yellow on a warning threshold and red on a critical condition.

- **Row 1** — `block` (+blocks/sec), `ver`, `up`, `sync` (+progress %), `peers`, `sessions`
- **Row 2** — `commits`, `votes`, `tmo`, `tc`, `lag` (exec behind consensus), `lag_evt`
- **Row 3** — `exec` (ledger block +diff), `tx`, `txpool`, `fwd`, `p99`, `rpc`
- **Row 4** — `key`, `net`, `stake`, `prop`, `leader` (prop/handled), `voted` (votes/handled)

### Keys

| Key | Action |
|-----|--------|
| `Ctrl+O` | Toggle the full log view |
| `Ctrl+E` | Toggle the full event log view |
| `Ctrl+H` | Toggle the parameter help panel |
| `Esc` | Close the current overlay |
| `Ctrl+C` | Quit the viewer (the daemon keeps running) |

## See also

- [`docs/metrics.md`](docs/metrics.md) — reference for every Prometheus metric used (and other node data sources).
