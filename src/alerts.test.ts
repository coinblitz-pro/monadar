import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createAlertProcessor } from './alerts'
import type { HistoryEntry, Snapshot, Warning } from './types'

const SNAP: Snapshot = {
  block: 0, blockDelta: null, bps: '?', lag: 0, execP99: null, peers: null, p99: null,
  rpcMs: 0, syncing: null, syncPct: null, uptime: '?', ver: '?', led24h: null, missed24h: null,
}

const RPC_DOWN: Warning = { level: 'crit', key: 'RPC_DOWN', short: 'RPC DOWN', detail: 'down', notify: 'msg' }
const NOT_VOTING: Warning = { level: 'crit', key: 'NOT_VOTING', short: 'NOT VOTING', detail: 'nv' }

function makeProc(delayTicks = 3) {
  const sent: string[] = []
  const history: HistoryEntry[] = []
  const proc = createAlertProcessor({
    delayTicks,
    history: { push: (e) => history.push(e) },
    notifier: { send: (_text, key) => sent.push(key) },
    now: () => 1000,
  })
  return { proc, sent, history }
}

test('DOWN warning notifies only after delay_ticks while unobservable', () => {
  const { proc, sent } = makeProc(3)
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  assert.deepEqual(sent, [], 'no alert before delay_ticks')
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  assert.deepEqual(sent, [ 'RPC_DOWN' ], 'alert fires on the third down tick')
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  assert.deepEqual(sent, [ 'RPC_DOWN' ], 'no repeat alert while still down')
})

test('pre-existing warning is frozen during blackout, resolves only on recovery', () => {
  const { proc, history } = makeProc(1)
  // NOT_VOTING active and notified.
  proc.processWarnings('t', [ NOT_VOTING ], SNAP, true)
  // Blackout: only DOWN is observable; NOT_VOTING must not resolve.
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  proc.processWarnings('t', [ RPC_DOWN ], SNAP, false)
  assert.equal(history.filter(e => e.level === 'resolved').length, 0, 'nothing resolves while blind')
  // Recovery with the node genuinely no longer NOT_VOTING: both DOWN and the
  // frozen NOT_VOTING resolve now.
  proc.processWarnings('t', [], SNAP, true)
  const resolved = history.filter(e => e.level === 'resolved').map(e => e.text)
  assert.ok(resolved.some(t => t.includes('RPC DOWN')), 'RPC DOWN resolves on recovery')
  assert.ok(resolved.some(t => t.includes('NOT VOTING')), 'frozen NOT VOTING resolves on recovery')
})
