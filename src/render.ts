import { BUILD_VERSION } from './build'
import { c } from './colors'
import { thresholds } from './config'
import { loadEvents } from './events'
import { renderTable } from './format'
import { eventIcon, renderHistory } from './history_render'
import { getRecent, recentCount } from './log'
import { ui } from './ui'
import { sizeRatio } from './bft_log'
import type { Cell, CounterCell, Frame, RoundAgg } from './types'

const EOL = '\x1b[K'

// Last rendered dashboard body (timestamp → history). Set by applyFrame(),
// reused by paint() on every keypress so overlays toggle without a new frame.
let lastBody = ''

// Txs in our committed blocks plus our block size relative to neighbouring
// network blocks (100% = same). Yellow below 80% once there are enough samples.
function txsVsNet(a: RoundAgg): string {
  const ratio = sizeRatio(a)
  if (ratio === null) return String(a.txs)
  const pct = `${Math.round(ratio * 100)}% net`
  return `${a.txs} ${ratio < 0.8 && a.sizeSamples >= 10 ? c.yellow(pct) : c.dim(pct)}`
}

function fmtCell(cell: CounterCell): string {
  const v = cell.value !== null ? String(cell.value) : '?'
  return `${v} Δ${cell.delta !== null ? cell.delta : '-'}`
}

// Build the colored dashboard body from a Frame (or an error line), using this
// client's own thresholds for coloring. Stores it for subsequent repaints.
export function applyFrame(f: Frame) {
  if (f.error === 'rpc') {
    lastBody = `${c.red('ERROR: RPC failed')}${EOL}\n` + renderHistory(f.history)
    return
  }
  const blockDeltaStr = f.blockDelta !== null ? String(f.blockDelta) : '-'
  if (f.error === 'metrics') {
    lastBody = `block: ${f.block} Δ${blockDeltaStr}  |  ${c.yellow('metrics unavailable')}${EOL}\n`
      + renderHistory(f.history)
    return
  }

  const syncVal = f.syncing === 1
    ? (f.syncPct ? c.red(`YES ${f.syncPct}%`) : c.red('YES!'))
    : c.green('no')

  const blockVal = f.blockDelta === 0
    ? c.red(`${f.block} Δ${blockDeltaStr}`)
    : `${f.block} Δ${blockDeltaStr}`

  const votesVal = f.votes.delta === 0 && f.blockDelta !== null && f.blockDelta > 0
    ? c.red(fmtCell(f.votes))
    : fmtCell(f.votes)

  // With round data, raw timeout/TC deltas are mostly other leaders' misses:
  // highlight tmo only when we were late on a round the network completed.
  const tmoBad = f.rounds
    ? f.rounds.lateTmo10m > 0
    : f.timeouts.delta !== null && f.timeouts.delta > 0
  const tmoVal = tmoBad ? c.yellow(fmtCell(f.timeouts)) : fmtCell(f.timeouts)

  const tcBad = !f.rounds && f.tc.delta !== null && f.tc.delta > thresholds.tc_max
  const tcVal = tcBad ? c.yellow(fmtCell(f.tc)) : fmtCell(f.tc)

  const lagEvtVal = f.lagEvtDelta !== null && f.lagEvtDelta > 0
    ? c.yellow(`Δ${f.lagEvtDelta}`)
    : `Δ${f.lagEvtDelta !== null ? f.lagEvtDelta : '-'}`

  // Prefer journal-based execution latency (p50/p99 ms); fall back to the
  // metric-based block lag when it's unavailable (or the daemon is older).
  const exec = f.exec ?? null
  let execLagVal: string
  if (exec) {
    const ms = `${exec.p50}/${exec.p99}ms`
    execLagVal = exec.p99 > thresholds.exec_ms_max ? c.yellow(ms) : ms
  } else {
    execLagVal = f.execLag > thresholds.lag_max
      ? c.yellow(String(f.execLag))
      : String(f.execLag)
  }

  const peersVal = f.peers !== null && f.peers < thresholds.peers_min
    ? c.yellow(String(f.peers))
    : String(f.peers ?? '?')

  const p99Val = f.p99 !== null && f.p99 > thresholds.p99_max
    ? c.yellow(`${f.p99}ms`)
    : `${f.p99 ?? '?'}ms`

  const rpcVal = f.rpcMs > thresholds.rpc_max
    ? c.yellow(`${f.rpcMs}ms`)
    : `${f.rpcMs}ms`

  let execVal: string
  if (f.execBlock !== null) {
    const diff = f.execBlock - f.block
    const lag = Math.max(0, f.block - f.execBlock)
    const diffStr = diff >= 0 ? `+${diff}` : String(diff)
    execVal = lag > thresholds.lag_max
      ? c.yellow(`${f.execBlock} (${diffStr})`)
      : `${f.execBlock} ${c.dim(`(${diffStr})`)}`
  } else {
    execVal = '?'
  }

  const propVal = f.proposals.delta !== null && f.proposals.delta > 0
    ? c.cyan(fmtCell(f.proposals))
    : fmtCell(f.proposals)

  let votedVal = '?'
  if (f.votedPct !== null) {
    votedVal = f.votedPct >= 99.9
      ? c.green(`${f.votedPct.toFixed(1)}%`)
      : f.votedPct < 95
        ? c.red(`${f.votedPct.toFixed(1)}%`)
        : c.yellow(`${f.votedPct.toFixed(1)}%`)
  }

  const row1: Cell[] = [
    { label: 'block:', value: `${blockVal} ${c.dim(`${f.bps}/s`)}` },
    { label: 'ver:', value: f.ver },
    { label: 'up:', value: f.uptime },
    { label: 'sync:', value: syncVal },
    { label: 'peers:', value: peersVal },
    { label: 'sessions:', value: String(f.sessions ?? '?') },
  ]
  const row2: Cell[] = [
    { label: 'commits:', value: fmtCell(f.commits) },
    { label: 'votes:', value: votesVal },
    { label: 'tmo:', value: tmoVal },
    { label: 'tc:', value: tcVal },
    { label: 'lag:', value: execLagVal },
    { label: 'lag_evt:', value: lagEvtVal },
  ]
  const row3: Cell[] = [
    { label: 'exec:', value: execVal },
    { label: 'tx:', value: fmtCell(f.tx) },
    { label: 'txpool:', value: String(f.txpool ?? '?') },
    { label: 'fwd:', value: fmtCell(f.fwd) },
    { label: 'p99:', value: p99Val },
    { label: 'rpc:', value: rpcVal },
  ]
  const row4: Cell[] = [
    { label: 'key:', value: c.cyan(f.key) },
    { label: 'net:', value: f.network },
    { label: 'stake:', value: f.stakePct },
    { label: 'prop:', value: propVal },
    { label: 'leader:', value: f.leaderPct },
    { label: 'voted:', value: votedVal },
  ]

  const rows = [ row1, row2, row3, row4 ]
  const r = f.rounds ?? null
  if (r) {
    const d = r.h24
    const lost = d.missed + d.failed
    const missedVal = r.missed10m > 0 ? c.red(String(lost)) : lost > 0 ? c.yellow(String(lost)) : String(lost)
    const lateStr = `late ${d.lateTmo}`
    const unseenVal = `${d.notReceived > 0 ? c.yellow(String(d.notReceived)) : '0'} `
      + (r.lateTmo10m > 0 ? c.yellow(lateStr) : c.dim(lateStr))
    const netPct = d.netRounds > 0 ? (d.netMissed / d.netRounds * 100).toFixed(2) + '%' : '?'
    rows.push([
      { label: 'led24h:', value: `${d.led} ${c.dim(`1h ${r.h1.led}`)}` },
      { label: 'missed:', value: missedVal },
      { label: 'unseen:', value: unseenVal },
      { label: 'blocks:', value: String(d.committed) },
      { label: 'txs:', value: txsVsNet(d) },
      { label: 'net_tc:', value: `${netPct} ${c.dim(`${r.roundsPerSec.toFixed(2)} r/s`)}` },
    ])
  }
  const lines = renderTable(rows)
  const status = f.status === 'crit' ? c.red('● CRITICAL')
    : f.status === 'warn' ? c.yellow('▲ WARNING ')
      : c.green('● OK      ')

  const versionMismatch = f.buildVersion !== BUILD_VERSION
  const versionHint = versionMismatch
    ? `  ${c.yellow('restart needed')} ${c.dim(`(daemon ${f.buildVersion ?? '?'} ≠ viewer ${BUILD_VERSION})`)}`
    : ''
  let body = `${c.cyan('monadar')}  ${c.dim(f.ts)}${versionHint}${EOL}\n${EOL}\n`
  for (const line of lines) {
    body += `${line}${EOL}\n`
  }
  body += `${EOL}\n`
  body += `${status}${EOL}\n`
  body += renderHistory(f.history)
  lastBody = body
}

function renderHelp(): string {
  let out = `${EOL}\n`
  const h = (l1: string, d1: string, l2?: string, d2?: string) => {
    const left = l1.padEnd(10) + ' ' + d1.padEnd(35)
    const right = l2 && d2 ? l2.padEnd(10) + ' ' + d2 : ''
    return c.dim(left + right) + EOL + '\n'
  }
  out += h('block', 'block number + rate (blocks/sec)', 'commits', 'consensus blocks committed')
  out += h('ver', 'node software version', 'votes', 'votes created by this node')
  out += h('up', 'node uptime', 'tmo', 'local timeouts (growth = problem)')
  out += h('sync', 'state sync (no = fully synced)', 'tc', 'timeout certificates')
  out += h('peers', 'discovered peers', 'lag', 'exec latency p50/p99 ms, last 60s')
  out += h('sessions', 'wireauth encrypted sessions', 'lag_evt', 'execution lagging events delta')
  out += h('exec', 'execution ledger block (+diff)', 'key', 'validator secp256k1 public key')
  out += h('tx', 'transactions committed', 'net', 'network name')
  out += h('txpool', 'transactions in pool', 'stake', 'node stake in basis points (%)')
  out += h('fwd', 'forwarded transactions received', 'prop', 'proposals created (was leader)')
  out += h('p99', 'UDP broadcast latency p99', 'leader', 'leader rate (prop/handled)')
  out += h('rpc', 'RPC response time', 'voted', 'vote participation (votes/handled)')
  out += h('led24h', 'rounds we led, 24h (1h)', 'missed', 'our rounds ended by TC, 24h')
  out += h('unseen', 'our rounds never seen; late = we timed out, net did not', 'blocks', 'our blocks committed, 24h')
  out += h('txs', 'txs in our blocks; % = size vs neighbours', 'net_tc', 'network rounds ended by TC, 24h')
  return out
}

// Word-wrap a long log line to the available width, breaking on spaces so
// key=val pairs stay intact; continuation lines are indented for readability.
function wrapLine(s: string, width: number): string[] {
  if (width < 10 || s.length <= width) return [ s ]
  const out: string[] = []
  let rest = s
  let indent = ''
  while (rest.length > width) {
    const limit = width - indent.length
    let cut = rest.lastIndexOf(' ', limit)
    if (cut <= 0) cut = limit
    out.push(indent + rest.slice(0, cut))
    rest = rest.slice(cut).replace(/^ +/, '')
    indent = '    '
  }
  if (rest) out.push(indent + rest)
  return out
}

function renderLog(): string {
  const rows = process.stdout.rows || 40
  const cols = process.stdout.columns || 120
  const avail = Math.max(5, rows - 5)
  // Wrap each entry, then keep the last `avail` visual lines (newest at bottom).
  const wrapped: string[] = []
  for (const l of getRecent(avail)) wrapped.push(...wrapLine(l, cols - 2))
  const shown = wrapped.slice(-avail)
  let out = `${EOL}\n`
  out += `${c.cyan('▌ Full log')}  ${c.dim(`(${recentCount()} lines · newest at bottom)`)}${EOL}\n`
  out += `${EOL}\n`
  if (shown.length === 0) {
    out += `${c.dim('(no log lines yet)')}${EOL}\n`
  } else {
    for (const l of shown) {
      out += ` ${c.dim(l)}${EOL}\n`
    }
  }
  return out
}

// Full persisted event log (warnings, resolves, restarts), newest at bottom.
// Reads the events file fresh each paint so it reflects the daemon's writes.
function renderEvents(): string {
  const rows = process.stdout.rows || 40
  const avail = Math.max(5, rows - 5)
  const all = loadEvents()
  let out = `${EOL}\n`
  out += `${c.cyan('▌ Events')}  ${c.dim(`(${all.length} total · newest at bottom)`)}${EOL}\n`
  out += `${EOL}\n`
  if (all.length === 0) {
    out += `${c.dim('(no events recorded — set log.file or log.events to persist)')}${EOL}\n`
  } else {
    for (const e of all.slice(-avail)) {
      const when = `${e.at.slice(5, 10)} ${e.at.slice(11, 19)}`
      out += ` ${c.dim(when)} ${eventIcon(e.level)} ${e.text}${EOL}\n`
    }
  }
  return out
}

function renderFooter(): string {
  const keys = ui.overlay === 'none'
    ? c.dim('ctrl+o log  ctrl+e events  ctrl+h help')
    : c.dim('esc close  ctrl+o log  ctrl+e events  ctrl+h help')
  let out = `${EOL}\n`
  out += `${keys}${EOL}\n`
  return out
}

// Compose and write the current frame: cursor home → body/overlay → footer →
// clear-to-end. Called after each frame and on every keypress for instant redraw.
export function paint() {
  let out = '\x1b[H'
  if (ui.overlay === 'log') {
    out += renderLog()
  } else if (ui.overlay === 'events') {
    out += renderEvents()
  } else {
    out += lastBody
    if (ui.overlay === 'help') out += renderHelp()
  }
  out += renderFooter()
  out += '\x1b[J'
  process.stdout.write(out)
}
