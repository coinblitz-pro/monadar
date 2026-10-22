import assert from 'node:assert/strict'
import { connect } from 'node:http2'
import test from 'node:test'
import { gzipSync } from 'node:zlib'
import { metricLabel, metricValue } from './metrics'
import { createSeriesStore, decodeExportRequest, startOtlpReceiver, unframe } from './otlp'

// Tiny protobuf encoder, just enough to build OTLP requests for the tests.
function varint(n: number | bigint): Buffer {
  let v = BigInt.asUintN(64, BigInt(n))
  const out: number[] = []
  do {
    let b = Number(v & 0x7fn)
    v >>= 7n
    if (v > 0n) b |= 0x80
    out.push(b)
  } while (v > 0n)
  return Buffer.from(out)
}
const tag = (no: number, wire: number) => varint((no << 3) | wire)
const len = (no: number, body: Buffer | string) => {
  const b = Buffer.isBuffer(body) ? body : Buffer.from(body)
  return Buffer.concat([ tag(no, 2), varint(b.length), b ])
}
const vint = (no: number, n: number) => Buffer.concat([ tag(no, 0), varint(n) ])
const f64 = (no: number, d: number) => {
  const b = Buffer.alloc(8)
  b.writeDoubleLE(d)
  return Buffer.concat([ tag(no, 1), b ])
}
const i64 = (no: number, n: number) => {
  const b = Buffer.alloc(8)
  b.writeBigInt64LE(BigInt(n))
  return Buffer.concat([ tag(no, 1), b ])
}
const keyValue = (k: string, v: string) => Buffer.concat([ len(1, k), len(2, len(1, v)) ])
const kv = (k: string, v: string) => len(1, keyValue(k, v)) // as Resource.attributes (field 1)

function request(): Buffer {
  const resource = Buffer.concat([
    kv('service.name', 'mainnet_node'),
    kv('service.version', '0.16.5'),
    kv('network', 'mainnet'),
  ])
  const gauge = len(2, Buffer.concat([
    len(1, 'monad.peer_disc.num_peers'),
    len(5, len(1, i64(6, 213))),
  ]))
  const cumulative = len(2, Buffer.concat([
    len(1, 'monad.state.consensus_events.commit_block'),
    len(7, Buffer.concat([ len(1, f64(4, 16461)), vint(2, 2), vint(3, 1) ])),
  ]))
  const delta = len(2, Buffer.concat([
    len(1, 'monad.delta_counter'),
    len(7, Buffer.concat([ len(1, Buffer.concat([ len(7, keyValue('kind', 'x')), i64(6, 5) ])), vint(2, 1), vint(3, 1) ])),
  ]))
  const histogram = len(2, Buffer.concat([ len(1, 'monad.some_histogram'), len(9, len(1, Buffer.alloc(0))) ]))
  const scope = len(2, Buffer.concat([ gauge, cumulative, delta, histogram ]))
  return len(1, Buffer.concat([ len(1, resource), scope ]))
}

test('decodeExportRequest reads gauges and sums with resource and point labels', () => {
  const points = decodeExportRequest(request())
  assert.deepEqual(points.map(p => [ p.name, p.value, p.delta ]), [
    [ 'monad_peer_disc_num_peers', 213, false ],
    [ 'monad_state_consensus_events_commit_block', 16461, false ],
    [ 'monad_delta_counter', 5, true ],
  ])
  assert.deepEqual(points[0].labels, {
    service_name: 'mainnet_node', service_version: '0.16.5', network: 'mainnet', job: 'mainnet_node',
  })
  assert.equal(points[2].labels.kind, 'x')
})

test('series store renders Prometheus text readable by metricValue/metricLabel', () => {
  const store = createSeriesStore({ secp_key: '035c51' })
  store.ingest(decodeExportRequest(request()), 0)
  store.ingest(decodeExportRequest(request()), 1000)
  const text = store.text(1000)
  assert.equal(metricValue(text, 'monad_peer_disc_num_peers'), 213)
  assert.equal(metricValue(text, 'monad_state_consensus_events_commit_block'), 16461)
  assert.equal(metricValue(text, 'monad_delta_counter'), 10, 'delta sums accumulate')
  assert.equal(metricLabel(text, 'monad_peer_disc_num_peers', 'secp_key'), '035c51')
  assert.equal(metricLabel(text, 'monad_peer_disc_num_peers', 'service_version'), '0.16.5')
  assert.equal(store.text(10 * 60_000), '', 'stale series expire')
})

test('unframe handles plain and gzip gRPC messages', () => {
  const msg = request()
  const frame = (flag: number, body: Buffer) => {
    const h = Buffer.alloc(5)
    h[0] = flag
    h.writeUInt32BE(body.length, 1)
    return Buffer.concat([ h, body ])
  }
  assert.deepEqual(unframe(frame(0, msg), undefined), [ msg ])
  assert.deepEqual(unframe(frame(1, gzipSync(msg)), 'gzip'), [ msg ])
})

test('receiver answers a gRPC Export call and yields null when the port is busy', async () => {
  const a = await startOtlpReceiver('127.0.0.1:47317')
  assert.ok(a)
  const b = await startOtlpReceiver('127.0.0.1:47317')
  assert.equal(b, null)

  // Real gRPC call against the first receiver.
  const client = connect('http://127.0.0.1:47317')
  const body = request()
  const framed = Buffer.concat([ Buffer.from([ 0, 0, 0, 0, 0 ]), body ])
  framed.writeUInt32BE(body.length, 1)
  const status = await new Promise<string | undefined>((resolve, reject) => {
    const req = client.request({
      ':method': 'POST',
      ':path': '/opentelemetry.proto.collector.metrics.v1.MetricsService/Export',
      'content-type': 'application/grpc',
      te: 'trailers',
    })
    req.on('trailers', (t) => resolve(t['grpc-status'] as string | undefined))
    req.on('error', reject)
    req.resume()
    req.end(framed)
  })
  client.close()
  assert.equal(status, '0')
  assert.equal(metricValue(a.text(), 'monad_peer_disc_num_peers'), 213)
  assert.ok(a.lastExport() !== null)
  a.close()
})
