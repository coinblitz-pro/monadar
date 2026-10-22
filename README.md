<p align="center">
  <img src="assets/logo.png" alt="monadar" width="160">
</p>

# monadar

Monitoring for a [Monad](https://monad.xyz) validator. One static binary runs as a daemon next
to the node: it reads the node's JSON-RPC (`eth_blockNumber`), receives its OTLP metrics, follows
the monad-bft and monad-execution journals, and sends Telegram alerts. A viewer attaches to the
daemon and shows a live dashboard.

```
monadar  18:48:12

block:   115663179 Δ2 3.28/s     ver:    0.16.5          up:     7h21m        sync:   no           peers:  209            sessions: 268
commits: 87042 Δ4                votes:  86989 Δ3        tmo:    259 Δ0       tc:     293 Δ0       lag:    9/25ms         lag_evt:  Δ0
exec:    115663177 (-2)          tx:     1621794 Δ93     txpool: 0            fwd:    23015 Δ0     p99:    183ms          rpc:      0ms
key:     035c51..6d4d            net:    mainnet         stake:  0.35%        prop:   330 Δ0       leader: 0.38%          voted:    100.0%
led24h:  4 1h 4                  missed: 0               unseen: 0 late 0     blocks: 4            txs:    75 97% net     net_tc:   0.00% 3.32 r/s

● OK

Recent events:
10-22 16:59:54 ● Node is not creating votes while blocks advance: consensus participation lost
10-22 16:59:55 ✓ Resolved: NOT VOTING (1s)
```

## What it watches

- Block production, execution lag and execution latency, peers, broadcast p99, RPC
- Our votes and leader rounds: proposed, missed, unseen, late, block size against the network
- Network health: share of rounds ended by timeout certificates
- Telegram alerts with a delivery queue that survives outages and restarts, plus `/status` and
  `/events` on demand
- A dead man's switch (`heartbeat_url`) for when the whole host goes down

## Requirements

- x86_64 Linux next to a Monad node (macOS works for development)
- RPC at `http://localhost:8080`
- Metrics: the node's `--otel-endpoint` pointing at `127.0.0.1:4317`, where monadar receives
  them (no OpenTelemetry Collector needed), or a Prometheus endpoint at
  `http://localhost:8889/metrics`
- Journal access (root or the `systemd-journal` group) for execution latency and leader rounds;
  rounds need monad-bft logging at DEBUG

## Install

Download a release binary, check it, and install it with its config next to it:

```sh
V=v1.0.1
curl -fLO https://github.com/coinblitz-pro/monadar/releases/download/$V/monadar-$V-x86_64-linux-musl
curl -fLO https://github.com/coinblitz-pro/monadar/releases/download/$V/SHA256SUMS
sha256sum -c SHA256SUMS
gh attestation verify monadar-$V-x86_64-linux-musl -R coinblitz-pro/monadar  # optional, see docs/building.md

sudo install -D -m 755 monadar-$V-x86_64-linux-musl /opt/monadar/monadar
sudo /opt/monadar/monadar --init                  # /opt/monadar/monadar.config.json
sudo /opt/monadar/monadar install --log /opt/monadar/logs/monadar.log
sudo systemctl enable --now monadar
/opt/monadar/monadar                              # attach the viewer
```

To update, replace the binary and run `sudo systemctl restart monadar`; the config, logs and
the Telegram queue stay. `install` refuses a binary in a temporary directory, because the unit
would point at a file that disappears.

## Usage

```sh
monadar                  # attach the viewer to the running daemon
monadar status           # is the daemon running?
monadar telegram-test    # send sample alerts to the configured chat
monadar --help
```

## Documentation

- [How it works](docs/architecture.md): daemon and viewer, update rate, data sources
- [Configuration](docs/configuration.md): commands, service setup, flags, config file, logs
- [Alerts and Telegram](docs/alerts.md): alert rules, delivery, bot commands
- [Leader rounds](docs/rounds.md): how rounds are classified, round alerts, block size
- [Dashboard](docs/dashboard.md): fields and keys
- [Metrics](docs/metrics.md): node metrics and journal lines, and which ones monadar uses
- [Building and releases](docs/building.md): build, tests, reproducible builds, verification
