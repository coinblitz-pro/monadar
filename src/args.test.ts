import assert from 'node:assert/strict'
import test from 'node:test'
import { parseArgs } from './args'

test('parseArgs does not treat another flag as --log value', () => {
  const args = parseArgs([ '--log', '--bell', '5' ])
  assert.equal(args.logFile, 'monadar.log')
  assert.equal(args.bell, true)
  assert.equal(args.intervalSec, 5)
})

test('parseArgs keeps install passthrough args for daemon service', () => {
  const args = parseArgs([ 'install', '--print', '--config', '/tmp/my config.json', '--socket', '/tmp/m.sock', '2' ])
  assert.equal(args.install, true)
  assert.equal(args.print, true)
  assert.deepEqual(args.servicePassthrough, [ '--config', '/tmp/my config.json', '--socket', '/tmp/m.sock', '2' ])
})

test('parseArgs parses threshold overrides without consuming positional interval', () => {
  const args = parseArgs([ '--peers-min', '0', '--p99-max', 'bad', '9' ])
  assert.deepEqual(args.thresholdOverrides, { peers_min: 0 })
  assert.equal(args.intervalSec, 9)
})
