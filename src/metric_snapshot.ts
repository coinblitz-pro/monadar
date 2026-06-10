import { metricLabel, metricValue } from './metrics'

export type MetricsSnapshot = {
  ver: string
  peers: number | null
  stakeBps: number | null
  sessions: number | null
  syncing: number | null
  uptimeUs: number | null
  commits: number | null
  votes: number | null
  timeouts: number | null
  tc: number | null
  lagging: number | null
  proposals: number | null
  handled: number | null
  execBlock: number | null
  txCommits: number | null
  txpool: number | null
  fwd: number | null
  latP99: number | null
  syncProgress: number | null
  syncTarget: number | null
  nodeKey: string
  network: string
}

export function readMetricsSnapshot(text: string): MetricsSnapshot {
  const v = (name: string) => metricValue(text, name)
  return {
    ver: metricLabel(text, 'monad_node_info', 'service_version'),
    peers: v('monad_peer_disc_num_peers'),
    stakeBps: v('monad_state_node_state_self_stake_bps'),
    sessions: v('monad_wireauth_udp_state_total_sessions'),
    syncing: v('monad_statesync_syncing'),
    uptimeUs: v('monad_total_uptime_us'),
    commits: v('monad_state_consensus_events_commit_block'),
    votes: v('monad_state_consensus_events_created_vote'),
    timeouts: v('monad_state_consensus_events_local_timeout'),
    tc: v('monad_state_consensus_events_created_tc'),
    lagging: v('monad_state_consensus_events_rx_execution_lagging'),
    proposals: v('monad_state_consensus_events_creating_proposal'),
    handled: v('monad_state_consensus_events_handle_proposal'),
    execBlock: v('monad_execution_ledger_block_num'),
    txCommits: v('monad_execution_ledger_num_tx_commits'),
    txpool: v('monad_bft_txpool_pool_tracked_txs'),
    fwd: v('monad_bft_txpool_pool_insert_forwarded_txs'),
    latP99: v('monad_bft_raptorcast_udp_primary_broadcast_latency_p99_ms'),
    syncProgress: v('monad_statesync_progress_estimate'),
    syncTarget: v('monad_statesync_last_target'),
    nodeKey: metricLabel(text, 'monad_node_info', 'secp_key'),
    network: metricLabel(text, 'monad_node_info', 'network'),
  }
}
