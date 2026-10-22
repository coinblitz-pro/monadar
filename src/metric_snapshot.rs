use crate::metrics::{metric_label, metric_value};

#[derive(Clone, Debug, PartialEq)]
pub struct MetricsSnapshot {
    pub ver: String,
    pub peers: Option<f64>,
    pub stake_bps: Option<f64>,
    pub sessions: Option<f64>,
    pub syncing: Option<f64>,
    pub uptime_us: Option<f64>,
    pub commits: Option<f64>,
    pub votes: Option<f64>,
    pub timeouts: Option<f64>,
    pub tc: Option<f64>,
    pub lagging: Option<f64>,
    pub proposals: Option<f64>,
    pub handled: Option<f64>,
    pub exec_block: Option<f64>,
    pub tx_commits: Option<f64>,
    pub txpool: Option<f64>,
    pub fwd: Option<f64>,
    pub lat_p99: Option<f64>,
    pub sync_progress: Option<f64>,
    pub sync_target: Option<f64>,
    pub node_key: String,
    pub network: String,
}

pub fn read_metrics_snapshot(text: &str) -> MetricsSnapshot {
    let v = |name: &str| metric_value(text, name);
    MetricsSnapshot {
        ver: metric_label(text, "monad_node_info", "service_version"),
        peers: v("monad_peer_disc_num_peers"),
        stake_bps: v("monad_state_node_state_self_stake_bps"),
        sessions: v("monad_wireauth_udp_state_total_sessions"),
        syncing: v("monad_statesync_syncing"),
        uptime_us: v("monad_total_uptime_us"),
        commits: v("monad_state_consensus_events_commit_block"),
        votes: v("monad_state_consensus_events_created_vote"),
        timeouts: v("monad_state_consensus_events_local_timeout"),
        tc: v("monad_state_consensus_events_created_tc"),
        lagging: v("monad_state_consensus_events_rx_execution_lagging"),
        proposals: v("monad_state_consensus_events_creating_proposal"),
        handled: v("monad_state_consensus_events_handle_proposal"),
        exec_block: v("monad_execution_ledger_block_num"),
        tx_commits: v("monad_execution_ledger_num_tx_commits"),
        txpool: v("monad_bft_txpool_pool_tracked_txs"),
        fwd: v("monad_bft_txpool_pool_insert_forwarded_txs"),
        lat_p99: v("monad_bft_raptorcast_udp_primary_broadcast_latency_p99_ms"),
        sync_progress: v("monad_statesync_progress_estimate"),
        sync_target: v("monad_statesync_last_target"),
        node_key: metric_label(text, "monad_node_info", "secp_key"),
        network: metric_label(text, "monad_node_info", "network"),
    }
}
