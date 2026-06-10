import assert from 'node:assert/strict'
import test from 'node:test'
import { metricLabel, metricValue } from './metrics'

test('metricValue reads labeled and unlabeled Prometheus samples', () => {
  const text = [
    '# HELP ignored ignored',
    'monad_peer_disc_num_peers{network="test"} 42 1710000000000',
    'monad_total_uptime_us 123000000',
  ].join('\n')

  assert.equal(metricValue(text, 'monad_peer_disc_num_peers'), 42)
  assert.equal(metricValue(text, 'monad_total_uptime_us'), 123000000)
})

test('metricValue ignores invalid numeric samples', () => {
  const text = [
    'metric_a 12',
    'metric_a NaN',
  ].join('\n')

  assert.equal(metricValue(text, 'metric_a'), 12)
})

test('metricLabel extracts requested labels from matching metric', () => {
  const text = 'monad_node_info{service_version="0.14.4",network="testnet"} 1'
  assert.equal(metricLabel(text, 'monad_node_info', 'network'), 'testnet')
})
