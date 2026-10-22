# Monad Node Metrics Reference

monad-bft and monad-rpc push metrics over OTLP/gRPC to `--otel-endpoint` every
`--record-metrics-interval-seconds` (1 s on our node). By default monadar receives them itself
on `127.0.0.1:4317`. The older setup runs an OpenTelemetry Collector there and exposes them at
`http://localhost:8889/metrics` in Prometheus text format; monadar can scrape that instead.

All BFT/consensus metrics are declared as `gauge` but most are monotonic counters (u64) that only increment. They reset to 0 on node restart.

Labels come from the OTLP resource: `service_name`, `service_version`, `network`. The collector's
Prometheus exporter adds `job` (= `service_name`), and its `resource` processor was configured to
add `secp_key`; monadar's receiver adds both the same way (`secp_key` from `node_key` in its
config). Below, names are given in the Prometheus form (dots replaced by underscores).

---

## Node Info

| Metric | Description |
|--------|-------------|
| `monad_node_info` | Always 1. Labels contain `service_version` (e.g. `0.12.7`) |
| `monad_total_uptime_us` | Node uptime in microseconds |

## Consensus (BFT)

| Metric | Description |
|--------|-------------|
| `monad_state_consensus_events_commit_block` | Blocks committed by consensus |
| `monad_state_consensus_events_created_vote` | Votes created by this node |
| `monad_state_consensus_events_created_qc` | Quorum certificates created |
| `monad_state_consensus_events_created_tc` | Timeout certificates created |
| `monad_state_consensus_events_created_nec` | No-endorsement certificates created |
| `monad_state_consensus_events_creating_proposal` | Proposals created (when this node is leader) |
| `monad_state_consensus_events_handle_proposal` | Proposals received and handled |
| `monad_state_consensus_events_handle_advance_round` | Round advances processed |
| `monad_state_consensus_events_handle_no_endorsement` | No-endorsement messages handled |
| `monad_state_consensus_events_handle_round_recovery` | Round recovery messages handled |
| `monad_state_consensus_events_enter_new_round_qc` | New rounds entered via QC |
| `monad_state_consensus_events_enter_new_round_tc` | New rounds entered via TC |
| `monad_state_consensus_events_local_timeout` | Local round timeouts (growth = problem) |
| `monad_state_consensus_events_remote_timeout_msg` | Remote timeout messages received |
| `monad_state_consensus_events_remote_timeout_msg_with_tc` | Remote timeout messages with TC |
| `monad_state_consensus_events_remote_timeout_msg_with_future_tc` | Remote timeout messages with future TC |
| `monad_state_consensus_events_old_remote_timeout` | Stale remote timeout messages (already past round) |
| `monad_state_consensus_events_vote_received` | Votes received from other validators |
| `monad_state_consensus_events_old_vote_received` | Stale votes received (already past round) |
| `monad_state_consensus_events_future_vote_received` | Future votes received (ahead of current round) |
| `monad_state_consensus_events_future_no_endorsement_received` | Future no-endorsement messages received |
| `monad_state_consensus_events_old_no_endorsement_received` | Stale no-endorsement messages received |
| `monad_state_consensus_events_process_qc` | QCs processed |
| `monad_state_consensus_events_process_tc` | TCs processed |
| `monad_state_consensus_events_process_old_qc` | Stale QCs received |
| `monad_state_consensus_events_process_old_tc` | Stale TCs received |
| `monad_state_consensus_events_proposal_with_tc` | Proposals that included a TC |
| `monad_state_consensus_events_out_of_order_proposals` | Out-of-order proposals received |
| `monad_state_consensus_events_invalid_proposal_round_leader` | Proposals from wrong leader |
| `monad_state_consensus_events_invalid_round_recovery_leader` | Round recovery from wrong leader |

## Execution Lag & Errors

| Metric | Description |
|--------|-------------|
| `monad_state_consensus_events_rx_execution_lagging` | **Monotonic counter**, NOT a gauge of current lag. Incremented when BFT checks execution result and gets `NotAvailableYet`, or when leader can't create proposal (no `eth_header`). Value ~15 after restart is normal. |
| `monad_state_consensus_events_rx_bad_state_root` | Bad state root detected |
| `monad_state_consensus_events_rx_base_fee_error` | Base fee calculation error |
| `monad_state_consensus_events_trigger_state_sync` | State sync triggered |
| `monad_state_consensus_events_failed_ts_validation` | Timestamp validation failures |
| `monad_state_consensus_events_failed_txn_validation` | Transaction validation failures |
| `monad_state_consensus_events_failed_verify_randao_reveal_sig` | RANDAO reveal signature verification failures |

## Validation Errors

| Metric | Description |
|--------|-------------|
| `monad_state_validation_errors_invalid_signature` | Invalid signature |
| `monad_state_validation_errors_invalid_author` | Invalid author |
| `monad_state_validation_errors_invalid_epoch` | Invalid epoch |
| `monad_state_validation_errors_invalid_seq_num` | Invalid sequence number |
| `monad_state_validation_errors_invalid_version` | Invalid version |
| `monad_state_validation_errors_invalid_vote_message` | Invalid vote message |
| `monad_state_validation_errors_invalid_tc_round` | Invalid TC round |
| `monad_state_validation_errors_insufficient_stake` | Insufficient stake for quorum |
| `monad_state_validation_errors_not_well_formed_sig` | Malformed signature |
| `monad_state_validation_errors_signatures_duplicate_node` | Duplicate node in signatures |
| `monad_state_validation_errors_val_data_unavailable` | Validator data unavailable |
| `monad_state_validation_errors_duplicate_tc_tip_round` | Duplicate TC tip round |
| `monad_state_validation_errors_empty_signers_tc_tip_round` | Empty signers in TC tip round |
| `monad_state_validation_errors_too_many_tc_tip_round` | Too many TC tip rounds |

## Execution Ledger

| Metric | Description |
|--------|-------------|
| `monad_execution_ledger_block_num` | The **finalized** block number, a true gauge set when monad-ledger handles `OptimisticCommit::Finalized` (monad-ledger/src/lib.rs). Normally 2 behind the proposed head, not the executed block. The collector prints it in scientific notation, e.g. `5.9422302e+07`. |
| `monad_execution_ledger_num_commits` | Number of ledger commits |
| `monad_execution_ledger_num_tx_commits` | Transactions in finalized blocks since bft started |

## Peer Discovery

| Metric | Description |
|--------|-------------|
| `monad_peer_disc_num_peers` | Number of discovered peers |
| `monad_peer_disc_num_pending_peers` | Peers pending discovery |
| `monad_peer_disc_num_downstream_fullnodes` | Downstream full nodes connected |
| `monad_peer_disc_send_ping` | Pings sent |
| `monad_peer_disc_send_pong` | Pongs sent |
| `monad_peer_disc_recv_ping` | Pings received |
| `monad_peer_disc_recv_pong` | Pongs received |
| `monad_peer_disc_drop_pong` | Pongs dropped |
| `monad_peer_disc_ping_timeout` | Ping timeouts |
| `monad_peer_disc_refresh` | Peer table refreshes |
| `monad_peer_disc_send_lookup_request` | Lookup requests sent |
| `monad_peer_disc_send_lookup_response` | Lookup responses sent |
| `monad_peer_disc_recv_lookup_request` | Lookup requests received |
| `monad_peer_disc_recv_lookup_response` | Lookup responses received |
| `monad_peer_disc_recv_open_lookup_request` | Open lookup requests received |
| `monad_peer_disc_recv_targeted_lookup_request` | Targeted lookup requests received |
| `monad_peer_disc_recv_raptorcast_request` | Raptorcast peer requests received |
| `monad_peer_disc_send_raptorcast_response` | Raptorcast peer responses sent |

## Staking

| Metric | Description |
|--------|-------------|
| `monad_state_node_state_self_stake_bps` | Node's own stake in basis points (43 = 0.43%) |

## State Sync

| Metric | Description |
|--------|-------------|
| `monad_statesync_syncing` | 1 = node is state-syncing, 0 = fully synced |
| `monad_statesync_progress_estimate` | Current statesync progress (used with `last_target` for %) |
| `monad_statesync_last_target` | Statesync target block (progress % = `progress_estimate / last_target * 100`) |
| `monad_statesync_server_pending_requests` | Pending state sync requests being served |
| `monad_statesync_server_num_syncdone_success` | Successful sync completions served |
| `monad_statesync_server_num_syncdone_failed` | Failed sync completions served |
| `monad_statesync_server_total_service_time_us` | Total time spent serving sync requests (us) |

## Block Sync

| Metric | Description |
|--------|-------------|
| `monad_state_blocksync_events_self_headers_request` | Header requests initiated by this node |
| `monad_state_blocksync_events_self_headers_response_successful` | Successful header responses (self-initiated) |
| `monad_state_blocksync_events_self_headers_response_failed` | Failed header responses (self-initiated) |
| `monad_state_blocksync_events_self_payload_request` | Payload requests initiated by this node |
| `monad_state_blocksync_events_self_payload_response_successful` | Successful payload responses (self-initiated) |
| `monad_state_blocksync_events_self_payload_response_failed` | Failed payload responses (self-initiated) |
| `monad_state_blocksync_events_self_payload_requests_in_flight` | Currently in-flight payload requests |
| `monad_state_blocksync_events_peer_headers_request` | Header requests from peers |
| `monad_state_blocksync_events_peer_headers_request_successful` | Successful header requests from peers |
| `monad_state_blocksync_events_peer_headers_request_failed` | Failed header requests from peers |
| `monad_state_blocksync_events_peer_payload_request` | Payload requests from peers |
| `monad_state_blocksync_events_peer_payload_request_successful` | Successful payload requests from peers |
| `monad_state_blocksync_events_peer_payload_request_failed` | Failed payload requests from peers |
| `monad_state_blocksync_events_headers_response_successful` | Successful header responses |
| `monad_state_blocksync_events_headers_response_failed` | Failed header responses |
| `monad_state_blocksync_events_headers_response_unexpected` | Unexpected header responses |
| `monad_state_blocksync_events_headers_validation_failed` | Header validation failures |
| `monad_state_blocksync_events_num_headers_received` | Total headers received |
| `monad_state_blocksync_events_payload_response_successful` | Successful payload responses |
| `monad_state_blocksync_events_payload_response_failed` | Failed payload responses |
| `monad_state_blocksync_events_payload_response_unexpected` | Unexpected payload responses |
| `monad_state_blocksync_events_request_timeout` | Block sync request timeouts |
| `monad_state_blocksync_events_request_failed_no_peers` | Requests failed due to no peers available |

## Block Tree

| Metric | Description |
|--------|-------------|
| `monad_state_blocktree_events_add_success` | Blocks added to blocktree |
| `monad_state_blocktree_events_add_dup` | Duplicate block additions |
| `monad_state_blocktree_events_prune_success` | Blocktree pruning operations |

## Transaction Pool

| Metric | Description |
|--------|-------------|
| `monad_bft_txpool_pool_tracked_txs` | Transactions currently tracked in pool |
| `monad_bft_txpool_pool_tracked_addresses` | Addresses currently tracked |
| `monad_bft_txpool_pool_insert_forwarded_txs` | Forwarded transactions inserted |
| `monad_bft_txpool_pool_insert_owned_txs` | Owned transactions inserted |
| `monad_bft_txpool_create_proposal` | Proposal creation requests |
| `monad_bft_txpool_create_proposal_elapsed_ns` | Proposal creation time (nanoseconds) |
| `monad_bft_txpool_pool_create_proposal` | Pool-level proposal creation |
| `monad_bft_txpool_pool_create_proposal_txs` | Transactions included in proposals |
| `monad_bft_txpool_pool_create_proposal_available_addresses` | Addresses available for proposal |
| `monad_bft_txpool_pool_create_proposal_tracked_addresses` | Tracked addresses at proposal time |
| `monad_bft_txpool_pool_create_proposal_backend_lookups` | Backend lookups during proposal |
| `monad_bft_txpool_pool_tracked_remove_committed_txs` | Committed transactions removed |
| `monad_bft_txpool_pool_tracked_remove_committed_addresses` | Committed addresses removed |
| `monad_bft_txpool_pool_tracked_evict_expired_txs` | Expired transactions evicted |
| `monad_bft_txpool_pool_tracked_evict_expired_addresses` | Expired addresses evicted |
| `monad_bft_txpool_pool_drop_nonce_too_low` | Dropped: nonce too low |
| `monad_bft_txpool_pool_drop_fee_too_low` | Dropped: fee too low |
| `monad_bft_txpool_pool_drop_pool_full` | Dropped: pool full |
| `monad_bft_txpool_pool_drop_pool_not_ready` | Dropped: pool not ready |
| `monad_bft_txpool_pool_drop_insufficient_balance` | Dropped: insufficient balance |
| `monad_bft_txpool_pool_drop_not_well_formed` | Dropped: malformed transaction |
| `monad_bft_txpool_pool_drop_existing_higher_priority` | Dropped: existing tx has higher priority |
| `monad_bft_txpool_pool_drop_replaced_by_higher_priority` | Dropped: replaced by higher priority tx |
| `monad_bft_txpool_pool_drop_internal_not_ready` | Dropped: internal not ready |
| `monad_bft_txpool_pool_drop_internal_state_backend_error` | Dropped: state backend error |
| `monad_bft_txpool_preload_backend_lookups` | Backend lookups during preload |
| `monad_bft_txpool_preload_backend_requests` | Backend requests during preload |
| `monad_bft_txpool_reject_forwarded_invalid_bytes` | Forwarded transactions rejected (invalid bytes) |

## Raptorcast (Broadcast Protocol)

| Metric | Description |
|--------|-------------|
| `monad_bft_raptorcast_udp_primary_broadcast_latency_p99_ms` | P99 UDP broadcast latency (ms) |
| `monad_bft_raptorcast_udp_primary_broadcast_latency_count` | Broadcast latency sample count |
| `monad_bft_raptorcast_secondary_publisher_current_group_size` | Secondary publisher group size |
| `monad_bft_raptorcast_secondary_publisher_sent_invites` | Secondary publisher invites sent |
| `monad_raptorcast_total_messages_received` | Total raptorcast messages received |
| `monad_raptorcast_total_recv_errors` | Total receive errors |
| `monad_raptorcast_auth_authenticated_udp_bytes_read` | Authenticated UDP bytes read |
| `monad_raptorcast_auth_authenticated_udp_bytes_written` | Authenticated UDP bytes written |
| `monad_raptorcast_auth_non_authenticated_udp_bytes_read` | Non-authenticated UDP bytes read |
| `monad_raptorcast_auth_non_authenticated_udp_bytes_written` | Non-authenticated UDP bytes written |
| `monad_raptorcast_decoding_cache_decoded` | Messages decoded |
| `monad_raptorcast_decoding_cache_decoded_hit` | Decoding cache hits |
| `monad_raptorcast_decoding_cache_pending_hit` | Pending cache hits |
| `monad_raptorcast_decoding_cache_new_entry` | New cache entries |
| `monad_raptorcast_decoding_cache_broadcast_total_insertions` | Broadcast cache insertions |
| `monad_raptorcast_decoding_cache_p2p_total_insertions` | P2P cache insertions |
| `monad_raptorcast_decoding_cache_validator_total_insertions` | Validator cache insertions |

## WireAuth (Encrypted Sessions)

| Metric | Description |
|--------|-------------|
| `monad_wireauth_udp_state_total_sessions` | Total active encrypted sessions |
| `monad_wireauth_udp_state_transport_sessions` | Transport-level sessions |
| `monad_wireauth_udp_state_sessions_by_public_key` | Sessions indexed by public key |
| `monad_wireauth_udp_state_sessions_by_socket` | Sessions indexed by socket |
| `monad_wireauth_udp_state_allocated_indices` | Allocated session indices |
| `monad_wireauth_udp_state_ip_session_counts_size` | Unique IPs with sessions |
| `monad_wireauth_udp_state_initiating_sessions` | Sessions currently initiating |
| `monad_wireauth_udp_state_responding_sessions` | Sessions currently responding |
| `monad_wireauth_udp_state_accepted_sessions_by_peer_size` | Accepted sessions by peer |
| `monad_wireauth_udp_state_initiated_session_by_peer_size` | Initiated sessions by peer |
| `monad_wireauth_udp_state_session_established_initiator` | Sessions established (as initiator) |
| `monad_wireauth_udp_state_session_established_responder` | Sessions established (as responder) |
| `monad_wireauth_udp_state_session_index_allocated` | Session indices allocated total |
| `monad_wireauth_udp_state_session_terminated` | Sessions terminated |
| `monad_wireauth_udp_state_packet_queue_size` | Packet queue size |
| `monad_wireauth_udp_state_timers_size` | Active session timers |
| `monad_wireauth_udp_api_connect` | Connect API calls |
| `monad_wireauth_udp_api_decrypt` | Decrypt API calls |
| `monad_wireauth_udp_api_encrypt_by_socket` | Encrypt-by-socket API calls |
| `monad_wireauth_udp_api_dispatch_control` | Dispatch control API calls |
| `monad_wireauth_udp_api_next_packet` | Next-packet API calls |
| `monad_wireauth_udp_api_tick` | Tick API calls |
| `monad_wireauth_udp_dispatch_handshake_initiation` | Handshake initiations dispatched |
| `monad_wireauth_udp_dispatch_handshake_response` | Handshake responses dispatched |
| `monad_wireauth_udp_dispatch_keepalive` | Keepalives dispatched |
| `monad_wireauth_udp_enqueued_handshake_init` | Handshake initiations enqueued |
| `monad_wireauth_udp_enqueued_handshake_response` | Handshake responses enqueued |
| `monad_wireauth_udp_enqueued_keepalive` | Keepalives enqueued |
| `monad_wireauth_udp_filter_pass` | Filter pass count |
| `monad_wireauth_udp_error_decrypt` | Decryption errors |
| `monad_wireauth_udp_error_decrypt_mac` | MAC decryption errors |
| `monad_wireauth_udp_error_dispatch_control` | Dispatch control errors |
| `monad_wireauth_udp_error_session_index_not_found` | Session index not found errors |

## Executor Timers

| Metric | Description |
|--------|-------------|
| `monad_executor_parent_total_exec_us` | Parent executor execution time (us) |
| `monad_executor_parent_total_poll_us` | Parent executor poll time (us) |
| `monad_executor_router_total_exec_us` | Router executor execution time (us) |
| `monad_executor_router_total_poll_us` | Router executor poll time (us) |
| `monad_executor_ledger_total_exec_us` | Ledger executor execution time (us) |
| `monad_executor_ledger_total_poll_us` | Ledger executor poll time (us) |
| `monad_executor_txpool_total_exec_us` | Txpool executor execution time (us) |
| `monad_executor_txpool_total_poll_us` | Txpool executor poll time (us) |
| `monad_executor_statesync_total_exec_us` | State sync executor execution time (us) |
| `monad_executor_statesync_total_poll_us` | State sync executor poll time (us) |
| `monad_executor_config_file_total_exec_us` | Config file executor execution time (us) |
| `monad_state_total_update_us` | Total state update time (us) |

## RPC Server (histogram)

These are standard Prometheus histograms with `_bucket`, `_sum`, `_count` suffixes.

| Metric | Description |
|--------|-------------|
| `monad_rpc_active_requests` | Currently in-flight HTTP requests (by `server_address`, `server_port`) |
| `monad_rpc_execution_duration_seconds` | RPC method execution duration. Labels: `main` (method name), `type` (`total` or `wait`) |
| `monad_rpc_request_duration_seconds` | Inbound HTTP request duration. Labels: `http_response_status_code`, `server_address` |

### Observed RPC methods

- `eth_blockNumber`: most frequent (monitoring polls)
- `eth_chainId`
- `eth_call`
- `eth_getBalance`
- `eth_getTransactionCount`
- `eth_sendRawTransaction`

### RPC notes

- `monad_rpc_active_requests` has many `server_address` entries: these are remote IPs that have connected to the node's RPC port, including scanners and bots
- Response codes 404 and 405 from external IPs are typically port scanners hitting the RPC port
- The `type="wait"` histogram tracks queue wait time before execution; `type="total"` includes both wait and execution

---

## Non-Prometheus data sources

### monad-ledger-tail (systemd service)

Separate service parsing ledger artifacts. JSON logs via journalctl:
```
journalctl -fu monad-ledger-tail
```

Three event types, all at INFO. monadar uses them for leader rounds (see [rounds.md](rounds.md)):

**`proposed_block`**: block proposed by consensus leader:
```json
{"message":"proposed_block", "round":"59510580", "parent_round":"59510579",
 "epoch":"1189", "seq_num":"59426425", "num_tx":"5", "gas_used":"12474726",
 "author":"02ae...", "block_ts_ms":"1772715239361", "now_ts_ms":"1772715239421",
 "author_address":"64.130.43.22:8000"}
```

**`finalized_block`**: block finalized (2 rounds after proposal):
```json
{"message":"finalized_block", "round":"59510579", "epoch":"1189",
 "seq_num":"59426424", "author":"0256...",
 "block_ts_ms":"1772715238920", "now_ts_ms":"1772715239756"}
```

**`timeout`**: a round that ended by timeout certificate; `author` is that round's leader (it
matched bft's `leader for round` for every TC in a 2-hour capture):
```json
{"message":"timeout", "round":"115808579", "author":"03bdde...", "now_ts_ms":"1792691817255",
 "author_address":"149.50.110.144:8000"}
```

Key derivable metrics:
- **Proposal delay**: `now_ts_ms - block_ts_ms` of `proposed_block` (p50 ~30 ms, p99 ~185 ms on mainnet, Oct 2026)
- **Finalization latency**: `now_ts_ms - block_ts_ms` of `finalized_block` (p50 ~620 ms, p99 ~770 ms)
- **Block time**: delta between consecutive `finalized_block` timestamps
- **Consensus gap**: `round - seq_num` (stable value = historical timeout rounds, growth = new timeouts)

### monad-mpt (CLI tool)

```
monad-mpt --storage /dev/triedb
```

Returns MonadDB disk usage: capacity, used %, history count, earliest/latest block, retention max, latest finalized/verified versions. Used by official `monad-status` script for `consensus.blockNumber`.

### External RPC comparison

Official `monad-status` script compares local block against external:
```
curl -s -X POST "https://rpc-{network}.monadinfra.com" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}'
```
Sync status: `in-sync` when `blockDifference <= 10`.

### monad-debug-node (control panel)

```
monad-debug-node -c /home/monad/monad-bft/controlpanel.sock get-peers
```

Returns actual peer info (alternative to `monad_peer_disc_num_peers` metric).

### BFT logs (journalctl)

```
journalctl -u monad-bft -f
```

Key parseable messages (JSON via journald). monadar uses the first eight; most are DEBUG. With
monad-ledger-tail running, bft can stay at INFO (see [rounds.md](rounds.md)):
- `"leader for round"`: `round`, `round_leader: "Some(<secp key>)"`
- `"proposal message"`: `author`, `proposal` (text with `proposal_round` and `seq_num`)
- `"advancing round"`: `certificate`: `Qc(...)` or `Tc(TimeoutCertificate { epoch, round, ...`
- `"committed block"` (INFO, target: `monad_ledger`): `num_tx`, `block_num`
- `"local timeout"`: `round`, `leader`: our pacemaker gave up on the round
- `"proposal stats"` (DEBUG, target: `monad_eth_block_validator`): `total_gas`, `proposal_size`, `txs`
- `"txpool executor received forwarded txs"`: `sender`, `num_txs`
- `"emitting create proposal command to txpool"`: `node_id`; only our own node logs it
- `"network has different vote/proposal pacing"` (WARN): node falling behind
- `"no eth_header found, can't propose"` (WARN): execution lagging event

journald delivers these lines about 0.2 s after RPC already shows the new block.

### Execution logs (journalctl)

```
journalctl -u monad-execution -f
```

Quill text format. Key parseable line: `__exec_block` with fields:
`bl` (block number), `id` (block hash), `ts` (ms, when BFT accepted the proposal; the line's own
timestamp minus `ts` is the execution latency monadar shows), `tx` (tx count), `rt` (retries), `rtp` (retry %),
`sr` (sender recovery us), `txe` (tx exec us), `cmt` (commit us), `tot` (total us),
`tpse` (TPS exec), `tps` (TPS overall), `gas` (gas used), `gpse` (GPS exec), `gps` (GPS overall)

### monlog (Category Labs tool)

```
# Install
curl -o /usr/local/bin/monlog https://pub-b0d0d7272c994851b4c8af22a766f571.r2.dev/scripts/monlog
chmod +x /usr/local/bin/monlog
# Usage
monlog        # scrapes BFT logs from last 60 seconds
monlog -r     # last 10 lines
monlog -n     # validator names instead of secp keys
```

Requires `monad` user in `systemd-journal` group.

### Systemd services

| Service | Purpose |
|---------|---------|
| `monad-bft` | BFT consensus |
| `monad-execution` | Block execution engine |
| `monad-rpc` | JSON-RPC server (port 8080) |
| `monad-ledger-tail` | Ledger event stream |
| `monad-cruft` | Periodic maintenance |
| `otelcol` | OpenTelemetry Collector (not needed with monadar's built-in OTLP receiver) |

### RaptorCast (full node block delivery)

Block propagation uses erasure-coded two-level broadcast:
- **Primary RaptorCast**: validators receive proposals from leader
- **Secondary RaptorCast**: validators push to downstream full nodes (~150 per group)

Key `node.toml` params: `max_group_size` (150), `round_span` (240), `max_num_group` (3), `raptor10_fullnode_redundancy_factor` (2.0)

Expected bandwidth at 10k TPS: ~6 MB/s (3 MB block data × 2x redundancy)

---

## Metrics used by monadar

Rare counters are shown as growth over a window (10 min, 1 h, 24 h, sampled once a minute):
their per-second deltas are 0 on almost every tick.

| Field | Source | Notes |
|--------------|--------|-------|
| header | `node_key` / bft journal / `secp_key` label, `monad_node_info` labels `network` and `service_version`, `monad_total_uptime_us`, `monad_state_node_state_self_stake_bps` | Values that rarely change. An uptime drop means the node restarted |
| block | `eth_blockNumber` (RPC) | The proposed head, ahead of execution and finalization |
| exec | `__exec_block` `bl` (execution journal), else `monad_execution_ledger_block_num` | Executed block and its diff to `block`. The metric fallback is the finalized height, marked `finalized` |
| exec_ms | `__exec_block` line time minus `ts` | Block execution time p50/p99 over 60 s. `ts` is when execution of the block started, so this is not the wait before execution |
| tps | `monad_execution_ledger_num_tx_commits` | Transactions per second over 30 s |
| sync | `monad_statesync_syncing` | Shown only while syncing. Once a node is live it cannot return to statesync (monad-statesync), so this matters at boot |
| voted | `created_vote / handle_proposal` | Over the last 10 min. A lifetime ratio barely moves after days of uptime |
| votes | `monad_state_consensus_events_created_vote` | Delta per tick; `NOT VOTING` watches it |
| tmo | `monad_state_consensus_events_local_timeout` | Last 10 min. Most follow other leaders' misses; `LATE` separates ours |
| leader row | ledger-tail, bft journal | See [rounds.md](rounds.md). Without round data: `prop` (`creating_proposal`) and our share of proposals |
| peers | `monad_peer_disc_num_peers` | |
| sessions | `monad_wireauth_udp_state_total_sessions` | |
| tc | ledger-tail `timeout` events, else `monad_state_consensus_events_created_tc` | Network rounds ended by TC, last 10 min |
| delay, final | ledger-tail | Proposal delay and time to finality, p50/p99 over 10 min |
| recv_p99 | `monad_bft_raptorcast_udp_primary_broadcast_latency_p99_ms` | Receive side: our clock minus the sender's timestamp when a broadcast message decodes, p99 over the node's 30 s tumbling window. Noisy |
| txpool | `monad_bft_txpool_pool_tracked_txs` | Current and max over 10 min; ~0 most of the time, txs leave the pool once any block includes them |
| fwd | `monad_bft_txpool_pool_insert_forwarded_txs` | Last 10 min; `/status` shows 1 h and 24 h |

Logged but not shown: `commit_block` (follows the block count), `rx_execution_lagging` (grows
only while we are leader and cannot propose, in practice right after a node start), RPC response
time (0 ms on localhost; `SLOW RPC` still alerts).
