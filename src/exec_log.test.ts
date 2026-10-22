import assert from 'node:assert/strict'
import test from 'node:test'
import { createExecWindow, parseExecBlockLine, summarize } from './exec_log'

const LINE = '2026-10-22 09:31:36.204673697 [118970] runloop_monad.cpp:381 LOG_INFO        '
  + '__exec_block,bl=115577020,id=0xb5435aaedcc0e2344f230f720998c4507e015527bb5b19779c1734bc93ec3b0a,'
  + 'ts=1792661496200,tx=   12,rt=  10,rtp=83.33%,sr=  143us,txe=  2479us,cmt=  1950us'

test('parseExecBlockLine extracts block and latency from an __exec_block line', () => {
  const s = parseExecBlockLine(LINE)
  assert.ok(s)
  assert.equal(s.block, 115577020)
  assert.equal(s.doneMs, Date.UTC(2026, 9, 22, 9, 31, 36) + 204.673697)
  // Epoch ms (~1.8e12) leaves ~0.2µs of double precision for the fraction.
  assert.ok(Math.abs(s.latencyMs - 4.673697) < 1e-3)
})

test('parseExecBlockLine ignores other execution log lines', () => {
  assert.equal(parseExecBlockLine('2026-10-22 09:31:36.204699535 [118970] runloop_monad.cpp:102 LOG_INFO        Run to block= 115577020'), null)
  assert.equal(parseExecBlockLine(''), null)
})

test('summarize computes percentiles and max', () => {
  const lat = Array.from({ length: 100 }, (_, i) => i + 1)
  assert.deepEqual(summarize(lat), { count: 100, p50: 51, p99: 100, max: 100 })
  assert.equal(summarize([]), null)
})

test('exec window drops samples older than the window', () => {
  const w = createExecWindow(1000)
  w.push({ block: 1, latencyMs: 500, doneMs: 0 })
  w.push({ block: 2, latencyMs: 5, doneMs: 900 })
  assert.deepEqual(w.snapshot(1500), { stats: { count: 1, p50: 5, p99: 5, max: 5 }, lastBlock: 2 })
  assert.deepEqual(w.snapshot(5000), { stats: null, lastBlock: null })
})
