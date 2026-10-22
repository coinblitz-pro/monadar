# Dashboard

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

Cells turn yellow on a warning threshold and red on a critical condition.

- **Row 1**: `block` (+blocks/sec), `ver`, `up`, `sync` (+progress %), `peers`, `sessions`
- **Row 2**: `commits`, `votes`, `tmo`, `tc`, `lag` (execution latency p50/p99 from the journal,
  or blocks behind consensus without it), `lag_evt`
- **Row 3**: `exec` (executed block, diff to `block`), `tx`, `txpool`, `fwd`, `p99`, `rpc`
- **Row 4**: `key`, `net`, `stake`, `prop`, `leader` (prop/handled), `voted` (votes/handled)
- **Row 5**: our leader rounds and network TCs (see [rounds.md](rounds.md)), when bft logs at DEBUG

Below: the status line and the last five events.

## Keys

| Key | Action |
|-----|--------|
| `Ctrl+O` | Toggle the full log view |
| `Ctrl+E` | Toggle the full event log view |
| `Ctrl+H` | Toggle the parameter help panel |
| `Esc` | Close the current overlay |
| `Ctrl+C` | Quit the viewer (the daemon keeps running) |
