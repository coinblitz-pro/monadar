# Leader rounds

Leader rounds come from two journals:

- **monad-ledger-tail** (INFO): `proposed_block` with its author, `timeout` for every round that
  ended by TC with that round's leader as `author`, and `finalized_block`. This is enough to
  classify our rounds, so monad-bft can log at INFO.
- **monad-bft** at DEBUG: `leader for round`, `proposal message`, `advancing round`,
  `committed block`, `local timeout`, `proposal stats`, forwarded txs.

While ledger-tail is live (a line in the last 30 s), proposals, TCs and commits come from it,
and the bft journal adds what only it has: the leader schedule (for `unseen`), our local
timeouts (`LATE`) and forwarded txs. Without ledger-tail, everything comes from the bft
journal, as before. On a 2-hour mainnet capture both sources gave the same counts for our
rounds, and each ledger-tail `timeout` named the same leader as bft's `leader for round`
(`src/fixture_replay.rs`).

A round is classified 10 rounds after it happened, so its timeout certificate has surely
arrived:

| Proposal from the leader | Timeout certificate for the round | Result |
|---|---|---|
| yes | no | `ok` |
| no | yes | `missed` (the leader's block never came) |
| yes | yes | `failed` (proposed but did not get enough votes) |
| no | no | `unseen` (the network moved on, but this node never saw the proposal) |

## On the dashboard

The `leader` row shows our rounds over 24h (with 1h next to it): `led`, `missed` (missed +
failed), `unseen`, the `txs` in our committed blocks and our block `size` against the network.
`unseen` and `late` show `?` while bft is not at DEBUG. Without any round source the row falls
back to the `creating_proposal` counter. The `network` row shows `tc`, network rounds ended by
TC in the last 10 minutes.

Counters are kept in memory. On start monadar reads the last 24h of the ledger-tail journal, so
they are complete right after a restart; with bft as the only source they start from zero.

From ledger-tail, p50/p99 over the last 10 minutes, in the `network` row:

- `delay`: how late proposals reach us, our clock minus the proposer's block timestamp. It
  grows before our node starts timing out on rounds (`LATE`).
- `final`: time from the block timestamp to finalization.

`/status` in Telegram shows the same numbers for 1h and 24h.

## Alerts

- `MISSED ROUND` (critical, sent to Telegram after `delay_ticks`) fires while any of our rounds
  missed or failed in the last 10 minutes.
- `LATE` (warning) fires when our node logged a `local timeout` in the last 10 minutes for a round
  the network still completed with a QC (no TC). That points to our own network or CPU, not to
  the leader. Needs bft at DEBUG; without it the rule is unknown.
- `NET DEGRADED` (warning) fires when the share of network rounds ended by TC in the last
  10 minutes exceeds `net_tc_pct_max`, or more than `net_tc_run_max` TC rounds came back to back.
  It is marked as not caused by our node.
- While round data is available, the metric-based `TMO`/`TC` warnings are off. Those counters
  also grow on every other leader's miss, so they mostly add noise. The `tmo` cell is highlighted
  only for `LATE`.

## Block size

Each of our proposals is compared with the median size of the 50 network blocks right before
it: gas used with ledger-tail, the tx count from the bft journal otherwise. Network load changes during the day, and the median ignores outlier blocks. The `txs` cell
shows the 24h ratio as `N% net` (100% = same size as neighbours, yellow below 80%). `/status`
shows the ratio for 1h and 24h, plus how many transactions other nodes forwarded to our txpool
(bft at DEBUG only). RPC has no mempool methods, so this forwarded stream is our view of the
mempool.
