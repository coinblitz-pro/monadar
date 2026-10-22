import { followJournal } from './journal'
import type { RoundAgg, RoundSummary } from './types'

// Leader-round accounting from monad-bft's DEBUG journal. Every round logs:
//   "leader for round"  {round, round_leader: "Some(<secp key>)"}  — who should propose
//   "proposal message"  {author, proposal: "... proposal_round: R ... seq_num: N ..."}
//   "advancing round"   {certificate: "Qc(QC { info: Vote { .. round: R }" | "Tc(TimeoutCertificate { epoch: E, round: R"}
//   "committed block"   {block_num, num_tx}
//   "local timeout"     {round, leader}  — our pacemaker gave up on the round
//   "proposal stats"    {txs, total_gas} — follows the "proposal message" it describes
//   "txpool executor received forwarded txs" {sender, num_txs}
// The round right after a TC has no "leader for round" line; its proposal's
// author is the leader. A round is classified once the chain is FINAL_LAG
// rounds past it, so its TC (logged when entering R+1) has surely arrived:
//   proposal, no TC → ok        no proposal, TC → missed
//   proposal + TC   → failed    neither         → not_received (we didn't see it)
// A local timeout on a round that got no TC means the network finished the
// round normally and only we were late — that's our problem (network/CPU).
// A local timeout on a TC round is just the leader's miss.

const FINAL_LAG = 10
const MINUTE = 60_000
const DAY_MINUTES = 24 * 60
const MISSED_ALERT_MINUTES = 10
const MAX_OUR_SEQS = 1000
const SIZE_NEIGHBOURS = 50

export type BftEvent =
  | { kind: 'leader'; round: number; leader: string | null; t: number }
  | { kind: 'proposal'; round: number; author: string; seq: number | null; t: number }
  | { kind: 'tc'; round: number; t: number }
  | { kind: 'local_tmo'; round: number; t: number }
  | { kind: 'commit'; block: number; txs: number; t: number }
  | { kind: 'stats'; txs: number; t: number }
  | { kind: 'fwd'; txs: number; t: number }

const MARKERS = [
  '"leader for round"', '"proposal message"', '"advancing round"', '"committed block"', '"local timeout"',
  '"proposal stats"', '"txpool executor received forwarded txs"',
]

export function parseBftLine(line: string): BftEvent | null {
  // Cheap substring check first: the journal carries ~100 lines/s, most irrelevant.
  if (!MARKERS.some(m => line.includes(m))) return null
  let j: { timestamp?: string; fields?: Record<string, unknown> }
  try {
    j = JSON.parse(line)
  } catch {
    return null
  }
  const f = j.fields ?? {}
  const t = Date.parse(j.timestamp ?? '')
  if (!Number.isFinite(t)) return null

  switch (f.message) {
    case 'leader for round': {
      const m = /^Some\((\w+)\)$/.exec(String(f.round_leader))
      return { kind: 'leader', round: Number(f.round), leader: m ? m[1] : null, t }
    }
    case 'proposal message': {
      const p = String(f.proposal)
      const r = /proposal_round: (\d+)/.exec(p)
      if (!r) return null
      const s = /seq_num: (\d+)/.exec(p)
      return { kind: 'proposal', round: Number(r[1]), author: String(f.author), seq: s ? Number(s[1]) : null, t }
    }
    case 'advancing round': {
      const m = /^Tc\(TimeoutCertificate \{ epoch: \d+, round: (\d+)/.exec(String(f.certificate))
      return m ? { kind: 'tc', round: Number(m[1]), t } : null
    }
    case 'committed block':
      return { kind: 'commit', block: Number(f.block_num), txs: Number(f.num_tx) || 0, t }
    case 'local timeout':
      return { kind: 'local_tmo', round: Number(f.round), t }
    case 'proposal stats':
      return { kind: 'stats', txs: Number(f.txs) || 0, t }
    case 'txpool executor received forwarded txs':
      return { kind: 'fwd', txs: Number(f.num_txs) || 0, t }
  }
  return null
}

type RoundInfo = { leader: string | null; author: string | null; seq: number | null; tc: boolean; localTmo: boolean; t: number }

export function emptyAgg(): RoundAgg {
  return {
    led: 0, ok: 0, missed: 0, failed: 0, notReceived: 0, committed: 0, txs: 0, netRounds: 0, netMissed: 0, lateTmo: 0,
    sizeOurTxs: 0, sizePeerTxs: 0, sizeSamples: 0, fwdTxs: 0,
  }
}

function median(xs: number[]): number {
  const s = [ ...xs ].sort((a, b) => a - b)
  const m = s.length >> 1
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2
}

// Our block size relative to the network: 1 = same as neighbouring blocks.
export function sizeRatio(a: RoundAgg): number | null {
  return a.sizeSamples > 0 && a.sizePeerTxs > 0 ? a.sizeOurTxs / a.sizePeerTxs : null
}

function addAgg(into: RoundAgg, a: RoundAgg) {
  for (const k of Object.keys(into) as (keyof RoundAgg)[]) into[k] += a[k]
}

export function createRoundTracker() {
  let self: string | null = null
  const pending = new Map<number, RoundInfo>()
  const buckets = new Map<number, RoundAgg>() // minute index → counters
  const ourSeqs = new Set<number>()
  let maxRound = 0
  let lastMissed: { round: number; t: number } | null = null
  let lastEventT: number | null = null
  let lastProposalAuthor: string | null = null
  const peerSizes: number[] = [] // txs of the last SIZE_NEIGHBOURS network blocks (not ours)
  // Runs of consecutive TC rounds (≥2): several leaders in a row produced no
  // block, so the whole network stood still for seconds.
  let lastFinal = 0
  let tcRun = 0
  const tcRuns: { start: number; len: number; t: number }[] = []

  function bucket(t: number): RoundAgg {
    const m = Math.floor(t / MINUTE)
    let b = buckets.get(m)
    if (!b) {
      b = emptyAgg()
      buckets.set(m, b)
      for (const k of buckets.keys()) if (k < m - DAY_MINUTES) buckets.delete(k)
    }
    return b
  }

  function info(round: number, t: number): RoundInfo {
    let r = pending.get(round)
    if (!r) {
      r = { leader: null, author: null, seq: null, tc: false, localTmo: false, t }
      pending.set(round, r)
    }
    return r
  }

  function finalize(round: number, r: RoundInfo) {
    const leader = r.leader ?? r.author
    const proposed = r.author !== null && r.author === leader
    const b = bucket(r.t)
    b.netRounds++
    if (r.tc) b.netMissed++
    if (r.localTmo && !r.tc) b.lateTmo++
    trackTcRun(round, r)
    if (!self || leader !== self) return

    b.led++
    if (proposed && !r.tc) {
      b.ok++
    } else if (proposed) {
      b.failed++
    } else if (r.tc) {
      b.missed++
    } else {
      b.notReceived++
    }
    if (r.tc) lastMissed = { round, t: r.t }
  }

  function trackTcRun(round: number, r: RoundInfo) {
    tcRun = r.tc ? (round === lastFinal + 1 ? tcRun : 0) + 1 : 0
    lastFinal = round
    if (tcRun < 2) return
    const start = round - tcRun + 1
    const last = tcRuns[tcRuns.length - 1]
    if (last && last.start === start) last.len = tcRun
    else tcRuns.push({ start, len: tcRun, t: r.t })
    while (tcRuns.length > 0 && tcRuns[0].t < r.t - DAY_MINUTES * MINUTE) tcRuns.shift()
  }

  function blockSize(txs: number, t: number) {
    const author = lastProposalAuthor
    lastProposalAuthor = null
    if (author === null) return
    if (self && author === self) {
      if (peerSizes.length < SIZE_NEIGHBOURS / 2) return // not enough context yet
      const b = bucket(t)
      b.sizeOurTxs += txs
      b.sizePeerTxs += median(peerSizes)
      b.sizeSamples++
    } else {
      peerSizes.push(txs)
      if (peerSizes.length > SIZE_NEIGHBOURS) peerSizes.shift()
    }
  }

  function ingest(e: BftEvent) {
    lastEventT = e.t
    if (e.kind === 'stats') return blockSize(e.txs, e.t)
    if (e.kind === 'fwd') {
      bucket(e.t).fwdTxs += e.txs
      return
    }
    if (e.kind === 'commit') {
      if (ourSeqs.delete(e.block)) {
        const b = bucket(e.t)
        b.committed++
        b.txs += e.txs
      }
      return
    }

    const r = info(e.round, e.t)
    if (e.kind === 'leader') r.leader = e.leader
    else if (e.kind === 'proposal') {
      r.author = e.author
      r.seq = e.seq
      lastProposalAuthor = e.author
      // Remember our block numbers now: "committed block" arrives a few rounds
      // later, well before the round is finalized.
      if (self && e.author === self && e.seq !== null) {
        ourSeqs.add(e.seq)
        if (ourSeqs.size > MAX_OUR_SEQS) ourSeqs.delete(ourSeqs.values().next().value!)
      }
    } else if (e.kind === 'local_tmo') r.localTmo = true
    else r.tc = true

    if (e.round > maxRound) {
      maxRound = e.round
      // Finalize in round order so consecutive-TC runs are detected correctly.
      const ready = [ ...pending.keys() ].filter(round => round <= maxRound - FINAL_LAG).sort((a, b) => a - b)
      for (const round of ready) {
        finalize(round, pending.get(round)!)
        pending.delete(round)
      }
    }
  }

  function sum(fromMinute: number, toMinute: number): RoundAgg {
    const a = emptyAgg()
    for (const [ m, b ] of buckets) if (m >= fromMinute && m <= toMinute) addAgg(a, b)
    return a
  }

  // null until the first bft line arrives (DEBUG logging off, or no journal access).
  function summary(nowMs = Date.now()): RoundSummary | null {
    if (lastEventT === null) return null
    const now = Math.floor(nowMs / MINUTE)
    const last10 = sum(now - MISSED_ALERT_MINUTES + 1, now)
    const prevMinute = sum(now - 1, now - 1)
    return {
      h1: sum(now - 59, now),
      h24: sum(now - DAY_MINUTES + 1, now),
      missed10m: last10.missed + last10.failed,
      lateTmo10m: last10.lateTmo,
      net10m: {
        rounds: last10.netRounds,
        tc: last10.netMissed,
        maxTcRun: Math.max(0, ...tcRuns.filter(x => x.t >= nowMs - MISSED_ALERT_MINUTES * MINUTE).map(x => x.len)),
      },
      lastMissed,
      roundsPerSec: prevMinute.netRounds / 60,
    }
  }

  return {
    ingest,
    summary,
    setSelf(key: string | null) {
      self = key && /^[0-9a-f]+$/i.test(key) ? key.toLowerCase() : null
    },
  }
}

export function startBftLogTail(tracker = createRoundTracker(), unit = 'monad-bft') {
  const journal = followJournal(unit, (line) => {
    const e = parseBftLine(line)
    if (e) tracker.ingest(e)
  })
  return { ...tracker, stop: journal.stop }
}
