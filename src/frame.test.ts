import assert from 'node:assert/strict'
import test from 'node:test'
import { buildTickFrame, createCollectorState, recordBlock } from './frame'
import type { MetricsSnapshot } from './metric_snapshot'
import type { Thresholds } from './types'

const NET_OK = { rounds: 1500, tc: 10, maxTcRun: 1 }
const thresholds: Thresholds = { peers_min: 50, p99_max: 500, lag_max: 5, rpc_max: 200, tc_max: 0, exec_ms_max: 250, stuck_sec: 5, net_tc_pct_max: 3, net_tc_run_max: 1 }

function metrics(overrides: Partial<MetricsSnapshot> = {}): MetricsSnapshot {
  return {
    ver: '0.14.4',
    peers: 100,
    stakeBps: 43,
    sessions: 10,
    syncing: 0,
    uptimeUs: 10_000_000,
    commits: 100,
    votes: 100,
    timeouts: 0,
    tc: 0,
    lagging: 0,
    proposals: 1,
    handled: 100,
    execBlock: 100,
    txCommits: 1000,
    txpool: 0,
    fwd: 0,
    latP99: 100,
    syncProgress: null,
    syncTarget: null,
    nodeKey: '0x1234567890abcdef',
    network: 'testnet',
    ...overrides,
  }
}

test('buildTickFrame computes execution lag as consensus minus execution block', () => {
  const state = createCollectorState()
  const sample = recordBlock(state, 110, 0)
  const { frame } = buildTickFrame(state, {
    ts: '12:00:00',
    block: 110,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics({ execBlock: 100 }),
    thresholds,
  })

  assert.equal(frame.execLag, 10)
  assert.equal(frame.status, 'warn')
  assert.deepEqual(frame.warnings.map(w => w.key), [ 'LAG' ])
})

test('buildTickFrame fallback lag grows when block delta exceeds commit delta', () => {
  const state = createCollectorState()

  let sample = recordBlock(state, 100, 0)
  buildTickFrame(state, {
    ts: '12:00:00',
    block: 100,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics({ execBlock: null, commits: 100 }),
    thresholds,
  })

  sample = recordBlock(state, 105, 1000)
  const { frame } = buildTickFrame(state, {
    ts: '12:00:01',
    block: 105,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics({ execBlock: null, commits: 102 }),
    thresholds,
  })

  assert.equal(frame.execLag, 3)
  assert.equal(frame.bps, '5.00')
})

test('buildTickFrame prefers the execution journal over the lagging metric', () => {
  const state = createCollectorState()
  const sample = recordBlock(state, 110, 0)
  const { frame, snap } = buildTickFrame(state, {
    ts: '12:00:00',
    block: 110,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics({ execBlock: 100 }),
    thresholds,
    exec: { stats: { count: 150, p50: 5, p99: 49, max: 136 }, lastBlock: 110 },
  })

  assert.equal(frame.execBlock, 110)
  assert.equal(frame.execLag, 0)
  assert.deepEqual(frame.exec, { count: 150, p50: 5, p99: 49, max: 136 })
  assert.equal(snap.execP99, 49)
  assert.equal(frame.status, 'ok')
  assert.match(frame.logLine, /exec_p50=5ms exec_p99=49ms exec_max=136ms exec_n=150/)
})

test('buildTickFrame raises a critical MISSED_ROUND when our rounds timed out recently', () => {
  const state = createCollectorState()
  const sample = recordBlock(state, 110, 0)
  const agg = { led: 3, ok: 2, missed: 1, failed: 0, notReceived: 0, committed: 2, txs: 20, netRounds: 1000, netMissed: 5, lateTmo: 0, sizeOurTxs: 0, sizePeerTxs: 0, sizeSamples: 0, fwdTxs: 0 }
  const { frame, snap } = buildTickFrame(state, {
    ts: '12:00:00',
    block: 110,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics({ execBlock: 110 }),
    thresholds,
    rounds: { h1: agg, h24: agg, missed10m: 1, lateTmo10m: 0, net10m: NET_OK, lastMissed: { round: 5, t: 0 }, roundsPerSec: 2.5 },
  })

  assert.deepEqual(frame.warnings.map(w => w.key), [ 'MISSED_ROUND' ])
  assert.equal(frame.status, 'crit')
  assert.equal(snap.led24h, 3)
  assert.equal(snap.missed24h, 1)
  assert.match(frame.logLine, /led24=3 ok24=2 missed24=1 failed24=0 unseen24=0 blocks24=2 txs24=20 net_tc24=5\/1000/)
})

test('with round data, network timeouts/TCs are not warnings but our late timeouts are', () => {
  const agg = { led: 0, ok: 0, missed: 0, failed: 0, notReceived: 0, committed: 0, txs: 0, netRounds: 1000, netMissed: 5, lateTmo: 0, sizeOurTxs: 0, sizePeerTxs: 0, sizeSamples: 0, fwdTxs: 0 }
  const tick = (lateTmo10m: number, withRounds: boolean) => {
    const state = createCollectorState()
    let sample = recordBlock(state, 110, 0)
    const base = { ts: '12:00:00', bps: sample.bps, rpcMs: 10, thresholds }
    buildTickFrame(state, { ...base, block: 110, blockDelta: sample.blockDelta, metrics: metrics({ execBlock: 110 }) })
    sample = recordBlock(state, 112, 1000)
    return buildTickFrame(state, {
      ...base,
      block: 112,
      blockDelta: sample.blockDelta,
      // another leader missed: both counters grew by one
      metrics: metrics({ execBlock: 112, timeouts: 1, tc: 1, votes: 101 }),
      rounds: withRounds ? { h1: agg, h24: agg, missed10m: 0, lateTmo10m, net10m: NET_OK, lastMissed: null, roundsPerSec: 2.5 } : null,
    }).frame.warnings.map(w => w.key)
  }

  assert.deepEqual(tick(0, false), [ 'TMO', 'TC' ], 'metric fallback without journal data')
  assert.deepEqual(tick(0, true), [], 'network misses are not our problem')
  assert.deepEqual(tick(2, true), [ 'LATE' ])
})

test('buildTickFrame warns when execution latency p99 exceeds exec_ms_max', () => {
  const state = createCollectorState()
  const sample = recordBlock(state, 110, 0)
  const { frame } = buildTickFrame(state, {
    ts: '12:00:00',
    block: 110,
    blockDelta: sample.blockDelta,
    bps: sample.bps,
    rpcMs: 10,
    metrics: metrics(),
    thresholds,
    exec: { stats: { count: 150, p50: 40, p99: 300, max: 900 }, lastBlock: 110 },
  })

  assert.deepEqual(frame.warnings.map(w => w.key), [ 'EXEC_SLOW' ])
})

test('NET_DEGRADED fires on an abnormal TC share or back-to-back TCs, marked as network', () => {
  const agg = { led: 0, ok: 0, missed: 0, failed: 0, notReceived: 0, committed: 0, txs: 0, netRounds: 1500, netMissed: 10, lateTmo: 0, sizeOurTxs: 0, sizePeerTxs: 0, sizeSamples: 0, fwdTxs: 0 }
  const warn = (net10m: { rounds: number; tc: number; maxTcRun: number }) => {
    const state = createCollectorState()
    const sample = recordBlock(state, 110, 0)
    return buildTickFrame(state, {
      ts: '12:00:00', block: 110, blockDelta: sample.blockDelta, bps: sample.bps, rpcMs: 10,
      metrics: metrics({ execBlock: 110 }), thresholds,
      rounds: { h1: agg, h24: agg, missed10m: 0, lateTmo10m: 0, net10m, lastMissed: null, roundsPerSec: 2.5 },
    }).frame.warnings.filter(w => w.key === 'NET_DEGRADED')
  }

  assert.deepEqual(warn({ rounds: 1500, tc: 10, maxTcRun: 1 }), [], 'normal: ~0.7%, single TCs')
  const pct = warn({ rounds: 1500, tc: 60, maxTcRun: 1 })
  assert.equal(pct[0]?.short, 'NET TC 4.0%')
  assert.equal(pct[0]?.level, 'warn')
  assert.match(pct[0]!.detail, /not our node/)
  assert.equal(warn({ rounds: 1500, tc: 12, maxTcRun: 3 })[0]?.short, 'NET TC ×3 in a row')
  assert.deepEqual(warn({ rounds: 50, tc: 10, maxTcRun: 1 }), [], 'too few rounds for a share')
})

test('STUCK needs the block to stay unchanged for stuck_sec, not a single tick', () => {
  const state = createCollectorState()
  const warningsAt = (block: number, nowMs: number) => {
    const sample = recordBlock(state, block, nowMs)
    return buildTickFrame(state, {
      ts: '12:00:00', block, blockDelta: sample.blockDelta, bps: sample.bps, rpcMs: 10,
      metrics: metrics({ execBlock: block }), thresholds, stalledMs: sample.stalledMs,
    }).frame.warnings.map(w => w.key)
  }
  warningsAt(100, 0)
  // A timed-out round: same block for 1.5s → not stuck.
  assert.deepEqual(warningsAt(100, 1500).filter(k => k === 'STUCK'), [])
  assert.deepEqual(warningsAt(100, 4000).filter(k => k === 'STUCK'), [])
  assert.deepEqual(warningsAt(100, 5000).filter(k => k === 'STUCK'), [ 'STUCK' ])
  assert.deepEqual(warningsAt(101, 6000).filter(k => k === 'STUCK'), [], 'clears once blocks advance')
})
