# Configuration

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

## Configuration

`monadar --init` writes a template `monadar.config.json`:

```json
{
  "telegram": { "token": "", "chat_id": "", "delay_ticks": 3, "cooldown_min": 5, "rich": true, "api_url": "" },
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

- **telegram**: bot `token` and `chat_id` for alerts. `delay_ticks`: a critical alert must
  persist this many ticks before notifying (and a recovery as many ticks before "resolved" is
  sent); `cooldown_min`: minimum minutes between two alerts for the same condition. `rich`: send
  Telegram Rich Messages (headings, tables; default `true`), `false` for classic HTML messages.
  `api_url`: Bot API base, `https://api.telegram.org` by default (set it for a local Bot API
  server or proxy).
- **thresholds**: same values as the threshold flags above; CLI flags override the config.
- **log**: `file` path, flush interval `flush_sec`, rotation (see below), and `events`, the
  path of the persistent event log (defaults to `events.log` next to `file`).
- **socket**: override the unix socket path.
- **cpu**: `taskset` CPU list for the daemon.
- **otlp**: address of the built-in OTLP/gRPC metrics receiver (default `127.0.0.1:4317`, the
  same as the nodes' `--otel-endpoint`). monad-bft and monad-rpc push metrics to it directly, so
  no OpenTelemetry Collector is needed. If the port is taken (an otelcol is still running),
  monadar scrapes `http://localhost:8889/metrics` instead. The choice is made at startup, so
  restart monadar after removing otelcol. Set `""` to always scrape.
- **node_key**: the node's public secp256k1 key, reported as the `secp_key` label (otelcol used
  to add it through its `resource` processor). Optional: monadar learns it from the bft journal.
- **heartbeat_url**: dead man's switch. The daemon GETs this URL every `heartbeat_sec` (default
  60) seconds, e.g. a [healthchecks.io](https://healthchecks.io) check or an Uptime Kuma push
  monitor. That service alerts when the pings stop. This is the only way to hear about the host,
  its network or monadar itself going down: monadar cannot report its own outage.

## Logging & rotation

With `--log` (or `log.file` set), every tick appends one `key=val` line to the log file. The
file is rotated by size:

- `log.max_size_mb` (default `50`): rotate when the file reaches this size. `<= 0` disables rotation.
- `log.keep` (default `5`): number of numbered backups to keep (`monadar.log.1` … `monadar.log.N`);
  on rotation the live file becomes `monadar.log.1`, older backups shift up, and the oldest is dropped.

The event log (`events.log`) is rotated with the same settings. Only its end is read: the last
few events at startup and as many as fit on screen in the **Ctrl+E** view.

The viewer also keeps the most recent lines in memory for the **Ctrl+O** log view, regardless
of whether a log file is configured.
