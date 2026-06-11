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

- Node.js 18+ (uses the built-in `fetch`)
- A reachable Monad node: RPC at `http://localhost:8080`, metrics at `http://localhost:8889/metrics`

## Build

```sh
npm install
npm run build        # bundles to dist/monadar.js (executable)
```

During development you can run directly with tsx:

```sh
npm run dev          # = tsx src/index.ts
npm test             # unit tests for parsing, frame math, metrics, service generation
npm run typecheck    # = tsc --noEmit
```

The examples below use `monadar` to mean `node dist/monadar.js` (or `npm run dev --`).

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
| `monadar --init` | Write a `monadar.config.json` template next to the binary |

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

## Configuration

`monadar --init` writes a template `monadar.config.json`:

```json
{
  "telegram": { "token": "", "chat_id": "", "delay_ticks": 3, "cooldown_min": 5 },
  "thresholds": { "peers_min": 50, "p99_max": 500, "lag_max": 5, "rpc_max": 200, "tc_max": 0 },
  "log": { "file": "", "flush_sec": 60, "max_size_mb": 50, "keep": 5, "events": "" },
  "socket": "",
  "cpu": ""
}
```

- **telegram** — bot `token` and `chat_id` for alerts. `delay_ticks`: a critical alert must
  persist this many ticks before notifying; `cooldown_min`: minimum minutes between repeats.
- **thresholds** — same values as the threshold flags above; CLI flags override the config.
- **log** — `file` path, flush interval `flush_sec`, rotation (see below), and `events` — the
  path of the persistent event log (defaults to `events.log` next to `file`).
- **socket** — override the unix socket path.
- **cpu** — `taskset` CPU list for the daemon.

## Logging & rotation

With `--log` (or `log.file` set), every tick appends one `key=val` line to the log file. The
file is rotated by size:

- `log.max_size_mb` (default `50`) — rotate when the file reaches this size. `<= 0` disables rotation.
- `log.keep` (default `5`) — number of numbered backups to keep (`monadar.log.1` … `monadar.log.N`);
  on rotation the live file becomes `monadar.log.1`, older backups shift up, and the oldest is dropped.

The viewer also keeps the most recent lines in memory for the **Ctrl+O** log view, regardless
of whether a log file is configured.

## Alerts & events

Warnings are derived each tick from the thresholds (low peers, high p99/RPC, execution lag,
timeouts/TCs) plus critical conditions (state-syncing, block not advancing, not voting). They
show inline (yellow/red cells), accumulate in the event history, and — for critical ones that
persist `delay_ticks` — are sent to Telegram.

A **node restart** is detected via a drop in `monad_total_uptime_us` (uptime is monotonic, so a
drop means the process restarted). On restart all node counters reset to 0, so the tick's
counter deltas are neutralized to avoid bogus values, the event is logged to the history, and a
one-off Telegram notification is sent.

Every event (warnings, resolves, restarts) is appended to a persistent **event log** so history
survives a daemon restart — on startup the daemon reloads the most recent events back into the
strip shown under the status line. The full event log is browsable in the viewer via **Ctrl+E**.

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
