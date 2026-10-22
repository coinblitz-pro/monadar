import assert from 'node:assert/strict'
import test from 'node:test'
import { createRoundTracker, parseBftLine, sizeRatio, type BftEvent } from './bft_log'

const ME = '035c51a98dbf03fe411cb723aa89d2e5ec764ece056cf08bc76de2bea2107e6d4d'
const OTHER = '02fbe48c47aa'
const T0 = Date.parse('2026-10-22T11:36:41.385386Z')

// Line shapes as monad-bft 0.16.5 logs them (proposal/certificate bodies trimmed).
const leaderLine = (round: number, key: string | null, ts = '2026-10-22T11:36:41.385386Z') => JSON.stringify({
  timestamp: ts, level: 'DEBUG',
  fields: { message: 'leader for round', round: String(round), round_leader: key ? `Some(${key})` : 'None' },
  target: 'monad_consensus_state',
})
const proposalLine = (round: number, author: string, seq: number) => JSON.stringify({
  timestamp: '2026-10-22T11:36:41.385256Z', level: 'DEBUG',
  fields: {
    message: 'proposal message', author,
    proposal: `ProposalMessage { proposal_round: ${round}, proposal_epoch: 2312, tip: ConsensusTip { block_header: ConsensusBlockHeader { author: ${author}, epoch: 2312, block_round: ${round}, seq_num: ${seq}, timestamp_ns: 1792667576967387110 } }, last_round_tc: None }`,
  },
  target: 'monad_consensus_state',
})
const advanceLine = (cert: string) => JSON.stringify({
  timestamp: '2026-10-22T11:13:10.169671Z', level: 'DEBUG',
  fields: { message: 'advancing round', certificate: cert },
  target: 'monad_consensus_state',
})
const commitLine = (block: number, txs: number) => JSON.stringify({
  timestamp: '2026-10-22T11:36:41.387355Z', level: 'INFO',
  fields: { message: 'committed block', num_tx: txs, block_num: block },
  target: 'monad_ledger',
})

test('parseBftLine reads leader, proposal, TC and commit lines', () => {
  assert.deepEqual(parseBftLine(leaderLine(115733163, ME)), { kind: 'leader', round: 115733163, leader: ME, t: T0 })
  assert.deepEqual(parseBftLine(leaderLine(5, null))?.kind === 'leader' && parseBftLine(leaderLine(5, null)), { kind: 'leader', round: 5, leader: null, t: T0 })
  assert.deepEqual(parseBftLine(proposalLine(115728526, OTHER, 115596946)), {
    kind: 'proposal', round: 115728526, author: OTHER, seq: 115596946, t: Date.parse('2026-10-22T11:36:41.385256Z'),
  })
  assert.deepEqual(
    parseBftLine(advanceLine('Tc(TimeoutCertificate { epoch: 2312, round: 115728525, tip_rounds: LimitedVec([]) })')),
    { kind: 'tc', round: 115728525, t: Date.parse('2026-10-22T11:13:10.169671Z') },
  )
  assert.deepEqual(parseBftLine(commitLine(115601547, 10)), { kind: 'commit', block: 115601547, txs: 10, t: Date.parse('2026-10-22T11:36:41.387355Z') })
  const localTmo = JSON.stringify({
    timestamp: '2026-10-22T11:14:37.142280Z', level: 'DEBUG',
    fields: { message: 'local timeout', round: '115728811', leader: OTHER, next_leader: ME },
    target: 'monad_consensus_state',
  })
  assert.deepEqual(parseBftLine(localTmo), { kind: 'local_tmo', round: 115728811, t: Date.parse('2026-10-22T11:14:37.142280Z') })
})

test('parseBftLine ignores QC advances, other messages and non-JSON', () => {
  assert.equal(parseBftLine(advanceLine('Qc(QC { info: Vote { id: 8b7d..b058, epoch: 2312, round: 115728483 } })')), null)
  assert.equal(parseBftLine(JSON.stringify({ timestamp: '2026-10-22T11:36:41Z', fields: { message: 'try vote' } })), null)
  assert.equal(parseBftLine('"leader for round" but not json'), null)
})

test('round tracker classifies our rounds and counts our committed blocks', () => {
  const tr = createRoundTracker()
  tr.setSelf(ME.toUpperCase())
  const t = T0
  const ev: BftEvent[] = [
    // 100: we lead and propose block 1000 → ok, later committed with 7 txs
    { kind: 'leader', round: 100, leader: ME, t },
    { kind: 'proposal', round: 100, author: ME, seq: 1000, t },
    { kind: 'commit', block: 1000, txs: 7, t },
    // 101: other leader never proposes → TC (network miss, not ours)
    { kind: 'leader', round: 101, leader: OTHER, t },
    { kind: 'tc', round: 101, t },
    // 102: after a TC there is no leader line; the author is the leader
    { kind: 'proposal', round: 102, author: OTHER, seq: 1001, t },
    // 103: we lead, no proposal, TC → missed
    { kind: 'leader', round: 103, leader: ME, t },
    { kind: 'tc', round: 103, t },
    // 104: we lead, our proposal is seen but the round still times out → failed
    { kind: 'leader', round: 104, leader: ME, t },
    { kind: 'proposal', round: 104, author: ME, seq: 1002, t },
    { kind: 'tc', round: 104, t },
    // 105: we lead, nothing seen, no TC → the network went on without us seeing it
    { kind: 'leader', round: 105, leader: ME, t },
    // push the chain far enough to finalize rounds 100-105
    { kind: 'leader', round: 115, leader: OTHER, t },
  ]
  for (const e of ev) tr.ingest(e)

  const s = tr.summary(t)
  assert.ok(s)
  assert.deepEqual(s.h24, { led: 4, ok: 1, missed: 1, failed: 1, notReceived: 1, committed: 1, txs: 7, netRounds: 6, netMissed: 3, lateTmo: 0, sizeOurTxs: 0, sizePeerTxs: 0, sizeSamples: 0, fwdTxs: 0 })
  assert.deepEqual(s.h1, s.h24)
  assert.equal(s.missed10m, 2)
  assert.deepEqual(s.lastMissed, { round: 104, t })
})

test('local timeouts count as late only when the network completed the round without a TC', () => {
  const tr = createRoundTracker()
  tr.setSelf(ME)
  const t = T0
  // 200: other leader missed → network TC; our local timeout is just the miss.
  tr.ingest({ kind: 'leader', round: 200, leader: OTHER, t })
  tr.ingest({ kind: 'local_tmo', round: 200, t })
  tr.ingest({ kind: 'tc', round: 200, t })
  // 201: proposal arrived and the round got a QC, but we had already timed out → late.
  tr.ingest({ kind: 'proposal', round: 201, author: OTHER, seq: 5, t })
  tr.ingest({ kind: 'local_tmo', round: 201, t })
  tr.ingest({ kind: 'leader', round: 220, leader: OTHER, t })

  const s = tr.summary(t)!
  assert.equal(s.h24.lateTmo, 1)
  assert.equal(s.lateTmo10m, 1)
  assert.equal(s.h24.netMissed, 1)
  assert.equal(s.missed10m, 0)
})

test('round tracker windows: misses older than 10 min stop alerting, older than 24h drop out', () => {
  const tr = createRoundTracker()
  tr.setSelf(ME)
  tr.ingest({ kind: 'leader', round: 1, leader: ME, t: T0 })
  tr.ingest({ kind: 'tc', round: 1, t: T0 })
  tr.ingest({ kind: 'leader', round: 20, leader: OTHER, t: T0 })

  assert.equal(tr.summary(T0)!.missed10m, 1)
  assert.equal(tr.summary(T0 + 11 * 60_000)!.missed10m, 0)
  assert.equal(tr.summary(T0 + 11 * 60_000)!.h24.missed, 1)
  assert.equal(tr.summary(T0 + 2 * 3600_000)!.h1.missed, 0)
  assert.equal(tr.summary(T0 + 25 * 3600_000)!.h24.missed, 0)
})

test('round tracker reports null before any bft line and ignores rounds while self is unknown', () => {
  const tr = createRoundTracker()
  assert.equal(tr.summary(T0), null)
  tr.ingest({ kind: 'leader', round: 1, leader: ME, t: T0 })
  tr.ingest({ kind: 'leader', round: 20, leader: OTHER, t: T0 })
  assert.equal(tr.summary(T0)!.h24.led, 0)
  assert.equal(tr.summary(T0)!.h24.netRounds, 1)
})

test('net10m counts TC rounds and the longest run of consecutive TCs', () => {
  const tr = createRoundTracker()
  const t = T0
  // Rounds 1-10: TCs at 3, 6, 7, 8 → one single TC and a run of 3.
  for (let round = 1; round <= 10; round++) {
    tr.ingest({ kind: 'leader', round, leader: OTHER, t })
    if ([ 3, 6, 7, 8 ].includes(round)) tr.ingest({ kind: 'tc', round, t })
  }
  // Events can arrive out of round order; finalization must still be ordered.
  tr.ingest({ kind: 'tc', round: 12, t })
  tr.ingest({ kind: 'leader', round: 11, leader: OTHER, t })
  tr.ingest({ kind: 'leader', round: 30, leader: OTHER, t })

  const s = tr.summary(t)!
  assert.deepEqual(s.net10m, { rounds: 12, tc: 5, maxTcRun: 3 })
  assert.equal(tr.summary(t + 11 * 60_000)!.net10m.maxTcRun, 0, 'old runs drop out of the 10 min window')
})

test('block size is compared with the median of preceding network blocks', () => {
  const tr = createRoundTracker()
  tr.setSelf(ME)
  const t = T0
  let round = 1000
  const block = (author: string, txs: number) => {
    tr.ingest({ kind: 'proposal', round: round++, author, seq: round, t })
    tr.ingest({ kind: 'stats', txs, t })
  }
  // Too little context: our first block is skipped.
  block(ME, 5)
  // 50 network blocks of 20 txs with two outliers that the median ignores.
  for (let i = 0; i < 48; i++) block(OTHER, 20)
  block(OTHER, 500)
  block(OTHER, 0)
  block(ME, 15)
  tr.ingest({ kind: 'fwd', txs: 3, t })
  tr.ingest({ kind: 'fwd', txs: 4, t })

  const a = tr.summary(t)!.h24
  assert.equal(a.sizeSamples, 1)
  assert.equal(a.sizeOurTxs, 15)
  assert.equal(a.sizePeerTxs, 20)
  assert.equal(sizeRatio(a), 0.75)
  assert.equal(a.fwdTxs, 7)
})
