# Leader rounds

Leader rounds come from monad-bft's DEBUG journal lines `leader for round`, `proposal message`,
`advancing round` and `committed block`. A round is classified 10 rounds after it happened, so
its timeout certificate (logged when entering the next round) has surely arrived:

| Proposal from the leader | Timeout certificate for the round | Result |
|---|---|---|
| yes | no | `ok` |
| no | yes | `missed` (the leader's block never came) |
| yes | yes | `failed` (proposed but did not get enough votes) |
| no | no | `unseen` (the network moved on, but this node never saw the proposal) |

## On the dashboard

The fifth dashboard row shows our rounds over 24h (with 1h next to it): `led24h`, `missed`
(missed + failed), `unseen`, committed `blocks` and their `txs`. It also shows `net_tc`, the
share of all network rounds that ended by TC, with rounds/s. If bft logs at INFO only, the row
is not shown. Counters are kept in memory and reset when monadar restarts.

`/status` in Telegram shows the same numbers for 1h and 24h.

## Alerts

- `MISSED ROUND` (critical, sent to Telegram after `delay_ticks`) fires while any of our rounds
  missed or failed in the last 10 minutes.
- `LATE` (warning) fires when our node logged a `local timeout` in the last 10 minutes for a round
  the network still completed with a QC (no TC). That points to our own network or CPU, not to
  the leader.
- `NET DEGRADED` (warning) fires when the share of network rounds ended by TC in the last
  10 minutes exceeds `net_tc_pct_max`, or more than `net_tc_run_max` TC rounds came back to back.
  It is marked as not caused by our node.
- While round data is available, the metric-based `TMO`/`TC` warnings are off. Those counters
  also grow on every other leader's miss, so they mostly add noise. The `tmo` cell is highlighted
  only for `LATE`.

## Block size

Each of our proposals is compared with the median tx count of the 50 network blocks right before
it. Network load changes during the day, and the median ignores outlier blocks. The `txs` cell
shows the 24h ratio as `N% net` (100% = same size as neighbours, yellow below 80%). `/status`
shows the ratio for 1h and 24h, plus how many transactions other nodes forwarded to our txpool.
RPC has no mempool methods, so this forwarded stream is our view of the mempool.
