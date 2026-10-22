import { createServer, type IncomingHttpHeaders, type ServerHttp2Session, type ServerHttp2Stream } from 'http2'
import { gunzipSync } from 'zlib'

// Minimal built-in OTLP/gRPC metrics receiver, so monadar works without an
// OpenTelemetry Collector. monad-bft and monad-rpc push metrics to
// --otel-endpoint (http://127.0.0.1:4317 by default); we accept
// MetricsService/Export, decode the protobuf by hand (only the fields we need)
// and keep the latest value of every gauge/sum series. text() renders them in
// Prometheus exposition format so metric_snapshot.ts works unchanged.

const EXPORT_PATH = '/opentelemetry.proto.collector.metrics.v1.MetricsService/Export'
const SERIES_TTL_MS = 5 * 60_000

export type OtlpPoint = { name: string; labels: Record<string, string>; value: number; delta: boolean }

// --- protobuf wire format ---------------------------------------------------

type Field = { no: number; wire: number; varint: bigint; bytes: Buffer; offset: number }

function* fields(buf: Buffer): Generator<Field> {
  let pos = 0
  const varint = (): bigint => {
    let result = 0n
    let shift = 0n
    while (true) {
      if (pos >= buf.length) throw new Error('protobuf: truncated varint')
      const b = buf[pos++]
      result |= BigInt(b & 0x7f) << shift
      if ((b & 0x80) === 0) return result
      shift += 7n
    }
  }
  while (pos < buf.length) {
    const key = Number(varint())
    const no = key >>> 3
    const wire = key & 7
    const f: Field = { no, wire, varint: 0n, bytes: buf.subarray(0, 0), offset: 0 }
    if (wire === 0) {
      f.varint = varint()
    } else if (wire === 1) {
      f.bytes = buf.subarray(pos, pos + 8)
      pos += 8
    } else if (wire === 2) {
      const len = Number(varint())
      f.bytes = buf.subarray(pos, pos + len)
      pos += len
    } else if (wire === 5) {
      f.bytes = buf.subarray(pos, pos + 4)
      pos += 4
    } else {
      throw new Error(`protobuf: unsupported wire type ${wire}`)
    }
    if (pos > buf.length) throw new Error('protobuf: truncated field')
    yield f
  }
}

// --- OTLP messages (opentelemetry/proto/metrics/v1/metrics.proto) ------------

// AnyValue: string=1, bool=2, int=3, double=4. Arrays/kvlists/bytes are skipped.
function anyValue(buf: Buffer): string | null {
  for (const f of fields(buf)) {
    if (f.no === 1 && f.wire === 2) return f.bytes.toString('utf-8')
    if (f.no === 2 && f.wire === 0) return f.varint !== 0n ? 'true' : 'false'
    if (f.no === 3 && f.wire === 0) return BigInt.asIntN(64, f.varint).toString()
    if (f.no === 4 && f.wire === 1) return String(f.bytes.readDoubleLE(0))
  }
  return null
}

// KeyValue: key=1, value=2
function keyValue(buf: Buffer, into: Record<string, string>) {
  let key = ''
  let value: string | null = null
  for (const f of fields(buf)) {
    if (f.no === 1 && f.wire === 2) key = f.bytes.toString('utf-8')
    else if (f.no === 2 && f.wire === 2) value = anyValue(f.bytes)
  }
  if (key && value !== null) into[sanitize(key)] = value
}

// NumberDataPoint: attributes=7, as_double=4 (double), as_int=6 (sfixed64)
function dataPoint(buf: Buffer, labels: Record<string, string>): { labels: Record<string, string>; value: number } | null {
  const own = { ...labels }
  let value: number | null = null
  for (const f of fields(buf)) {
    if (f.no === 7 && f.wire === 2) keyValue(f.bytes, own)
    else if (f.no === 4 && f.wire === 1) value = f.bytes.readDoubleLE(0)
    else if (f.no === 6 && f.wire === 1) value = Number(f.bytes.readBigInt64LE(0))
  }
  return value === null ? null : { labels: own, value }
}

// Metric: name=1, gauge=5, sum=7 (histograms etc. are ignored).
// Gauge/Sum: data_points=1; Sum: aggregation_temporality=2 (1 = DELTA), is_monotonic=3.
function metric(buf: Buffer, labels: Record<string, string>, out: OtlpPoint[]) {
  let name = ''
  let data: Buffer | null = null
  let isSum = false
  for (const f of fields(buf)) {
    if (f.no === 1 && f.wire === 2) name = sanitize(f.bytes.toString('utf-8'))
    else if ((f.no === 5 || f.no === 7) && f.wire === 2) {
      data = f.bytes
      isSum = f.no === 7
    }
  }
  if (!name || !data) return

  const points: Buffer[] = []
  let deltaTemporality = false
  let monotonic = false
  for (const f of fields(data)) {
    if (f.no === 1 && f.wire === 2) points.push(f.bytes)
    else if (isSum && f.no === 2 && f.wire === 0) deltaTemporality = f.varint === 1n
    else if (isSum && f.no === 3 && f.wire === 0) monotonic = f.varint !== 0n
  }
  for (const p of points) {
    const dp = dataPoint(p, labels)
    if (dp) out.push({ name, labels: dp.labels, value: dp.value, delta: deltaTemporality && monotonic })
  }
}

// ExportMetricsServiceRequest: resource_metrics=1
// ResourceMetrics: resource=1 (Resource: attributes=1), scope_metrics=2
// ScopeMetrics: metrics=2
export function decodeExportRequest(buf: Buffer): OtlpPoint[] {
  const out: OtlpPoint[] = []
  for (const rm of fields(buf)) {
    if (rm.no !== 1 || rm.wire !== 2) continue
    const labels: Record<string, string> = {}
    const scopes: Buffer[] = []
    for (const f of fields(rm.bytes)) {
      if (f.no === 1 && f.wire === 2) {
        for (const a of fields(f.bytes)) if (a.no === 1 && a.wire === 2) keyValue(a.bytes, labels)
      } else if (f.no === 2 && f.wire === 2) {
        scopes.push(f.bytes)
      }
    }
    // Match the collector's Prometheus exporter: service.name is also exposed as job.
    if (labels.service_name && !labels.job) labels.job = labels.service_name
    for (const s of scopes) {
      for (const f of fields(s)) if (f.no === 2 && f.wire === 2) metric(f.bytes, labels, out)
    }
  }
  return out
}

// Same normalization as the collector's Prometheus exporter: `monad.peer_disc.num_peers` → `monad_peer_disc_num_peers`.
function sanitize(s: string): string {
  return s.replace(/[^a-zA-Z0-9_]/g, '_')
}

// --- series store -----------------------------------------------------------

function escapeLabel(v: string): string {
  return v.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')
}

export function createSeriesStore(extraLabels: Record<string, string> = {}) {
  const series = new Map<string, { name: string; labels: Record<string, string>; value: number; updated: number }>()
  let lastExport: number | null = null

  function ingest(points: OtlpPoint[], nowMs = Date.now()) {
    for (const p of points) {
      const labels = { ...extraLabels, ...p.labels }
      const key = p.name + JSON.stringify(Object.entries(labels).sort())
      const prev = series.get(key)
      const value = p.delta && prev ? prev.value + p.value : p.value
      series.set(key, { name: p.name, labels, value, updated: nowMs })
    }
    lastExport = nowMs
  }

  function text(nowMs = Date.now()): string {
    let out = ''
    for (const [ key, s ] of series) {
      if (nowMs - s.updated > SERIES_TTL_MS) {
        series.delete(key)
        continue
      }
      const labels = Object.entries(s.labels).map(([ k, v ]) => `${k}="${escapeLabel(v)}"`).join(',')
      out += `${s.name}{${labels}} ${s.value}\n`
    }
    return out
  }

  return { ingest, text, lastExport: () => lastExport }
}

// --- gRPC server --------------------------------------------------------------

// gRPC message framing: 1 byte compressed flag + 4 bytes big-endian length.
export function unframe(body: Buffer, encoding: string | undefined): Buffer[] {
  const msgs: Buffer[] = []
  let pos = 0
  while (pos + 5 <= body.length) {
    const compressed = body[pos] === 1
    const len = body.readUInt32BE(pos + 1)
    let msg = body.subarray(pos + 5, pos + 5 + len)
    pos += 5 + len
    if (compressed) {
      if (encoding !== 'gzip') throw new Error(`unsupported grpc-encoding ${encoding}`)
      msg = gunzipSync(msg)
    }
    msgs.push(msg)
  }
  return msgs
}

function grpcReply(stream: ServerHttp2Stream, status: number, message?: string) {
  if (stream.destroyed) return
  stream.respond({ ':status': 200, 'content-type': 'application/grpc' }, { waitForTrailers: true })
  stream.on('wantTrailers', () => {
    stream.sendTrailers({ 'grpc-status': String(status), ...(message ? { 'grpc-message': encodeURIComponent(message) } : {}) })
  })
  // Empty ExportMetricsServiceResponse on success; no body on error.
  stream.end(status === 0 ? Buffer.alloc(5) : undefined)
}

export type OtlpReceiver = {
  text(): string
  lastExport(): number | null
  close(): void
}

// Resolves to null if the address is taken (e.g. an otelcol is still running),
// so the caller can fall back to scraping its Prometheus endpoint.
export function startOtlpReceiver(listen: string, extraLabels: Record<string, string> = {}): Promise<OtlpReceiver | null> {
  const store = createSeriesStore(extraLabels)
  const i = listen.lastIndexOf(':')
  const host = listen.slice(0, i) || '127.0.0.1'
  const port = Number(listen.slice(i + 1))

  const server = createServer()
  server.on('stream', (stream: ServerHttp2Stream, headers: IncomingHttpHeaders) => {
    const chunks: Buffer[] = []
    stream.on('data', (c: Buffer) => chunks.push(c))
    stream.on('end', () => {
      // 12 = UNIMPLEMENTED (e.g. trace/log exporters), same as a metrics-only collector.
      if (headers[':method'] !== 'POST' || headers[':path'] !== EXPORT_PATH) return grpcReply(stream, 12, 'only metrics are supported')
      try {
        const encoding = headers['grpc-encoding'] as string | undefined
        for (const msg of unframe(Buffer.concat(chunks), encoding)) store.ingest(decodeExportRequest(msg))
        grpcReply(stream, 0)
      } catch (e) {
        grpcReply(stream, 3, (e as Error).message) // 3 = INVALID_ARGUMENT
      }
    })
    stream.on('error', () => {})
  })
  server.on('sessionError', () => {})
  // Exporters keep their HTTP/2 connection open; track sessions so close() can drop them.
  const sessions = new Set<ServerHttp2Session>()
  server.on('session', (s: ServerHttp2Session) => {
    sessions.add(s)
    s.on('close', () => sessions.delete(s))
  })

  return new Promise((resolve) => {
    server.once('error', () => resolve(null))
    server.listen(port, host, () => {
      server.on('error', () => {})
      resolve({
        text: () => store.text(),
        lastExport: store.lastExport,
        close() {
          server.close()
          for (const s of sessions) s.destroy()
        },
      })
    })
  })
}
