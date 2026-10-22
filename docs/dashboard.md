# Dashboard

```
monadar  21:50:13  035c51..6d4d · mainnet · 0.16.5 · up 10h23m · stake 0.35%

╭─ chain ─────────────────╮ ╭─ votes ───────────╮ ╭─ leader · 24h ──╮ ╭─ network ───────────╮ ╭─ mempool ───────╮
│ block    115699304 Δ3   │ │ voted  100.0% 10m │ │ led     499     │ │ peers     207       │ │ txpool  0       │
│ rate     3.31 blocks/s  │ │ votes  3.31 /s    │ │ led 1h  45      │ │ sessions  209       │ │ max     42 10m  │
│ exec     115699304 (+0) │ │ tmo    0 10m      │ │ missed  0       │ │ tc        0 10m     │ │ fwd     480 10m │
│ exec_ms  6/29ms         │ │                   │ │ txs     8874    │ │ delay     33/188ms  │ │                 │
│ tps      64             │ │                   │ │ size    99% net │ │ final     622/760ms │ │                 │
│                         │ │                   │ │                 │ │ recv_p99  170ms     │ │                 │
╰─────────────────────────╯ ╰───────────────────╯ ╰─────────────────╯ ╰─────────────────────╯ ╰─────────────────╯

● OK

Recent events:
10-22 16:59:54 ● Node is not creating votes while blocks advance: consensus participation lost
10-22 16:59:55 ✓ Resolved: NOT VOTING (1s)
```

The header holds what rarely changes: our key, network, node version, uptime and stake. Below
it, one panel per question. Panels sit side by side and wrap onto another band in a narrow
terminal. Values turn yellow on a warning threshold and red on a critical condition. Press
**Ctrl+H** for a description of every value.

- **chain**: is the node keeping up. `block` (proposed head, Δ per tick), `rate` (blocks/s), `exec`
  (executed block and its diff to `block`; `finalized` when it comes from the metric
  fallback), `exec_ms` (block execution time p50/p99, 60 s), `tps`. `sync` appears only while
  the node state-syncs, `rpc` only when it is slow.
- **votes**: are we voting. `voted` (votes per proposal handled, 10 min), `votes` (per second;
  one per block, so it matches `rate` when healthy), `tmo` (our local timeouts, 10 min).
- **leader · 24h**: our leader rounds. `led`, `led 1h`, `missed`, `unseen` and `late` (shown
  only while bft logs at DEBUG), `txs` in our blocks, `size` against the network. See
  [rounds.md](rounds.md).
- **network**: the network around us. `peers`, `sessions`, `tc` (rounds ended by TC, 10 min;
  `/status` adds the share),
  `delay` (how late proposals reach us), `final` (time to finality), `recv_p99` (receive
  latency of broadcast messages).
- **mempool**: are transactions reaching us. `txpool` (now), `max` (largest pool over 10 min;
  the pool drains within a second, so the max is what matters), `fwd` (txs forwarded to us,
  10 min).

Rare counters are shown over a window: their per-tick deltas are 0 almost every tick. Below the
panels: the status line and the last five events.

## Keys

| Key | Action |
|-----|--------|
| `Ctrl+O` | Toggle the full log view |
| `Ctrl+E` | Toggle the full event log view |
| `Ctrl+H` | Toggle the parameter help panel |
| `Esc` | Close the current overlay |
| `Ctrl+C` | Quit the viewer (the daemon keeps running) |
