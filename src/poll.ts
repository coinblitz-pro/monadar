import { METRICS_URL, RPC_URL } from './config'

type FetchFn = typeof fetch

function isBlockNumberResponse(value: unknown): value is { result: string } {
  return typeof value === 'object'
    && value !== null
    && typeof (value as { result?: unknown }).result === 'string'
    && /^0x[0-9a-fA-F]+$/.test((value as { result: string }).result)
}

export async function fetchBlockNumber(
  fetchFn: FetchFn = fetch,
  rpcUrl = RPC_URL,
  timeoutMs = 5000,
): Promise<{ block: number; rpcMs: number }> {
  const t0 = Date.now()
  const resp = await fetchFn(rpcUrl, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', method: 'eth_blockNumber', params: [], id: 1 }),
    signal: AbortSignal.timeout(timeoutMs),
  })
  if (!resp.ok) throw new Error(`RPC HTTP ${resp.status}`)
  const json = await resp.json()
  if (!isBlockNumberResponse(json)) throw new Error('RPC returned an invalid eth_blockNumber response')
  const block = parseInt(json.result, 16)
  if (!Number.isSafeInteger(block)) throw new Error(`RPC block number is out of range: ${json.result}`)
  return { block, rpcMs: Date.now() - t0 }
}

export async function fetchMetricsText(
  fetchFn: FetchFn = fetch,
  metricsUrl = METRICS_URL,
  timeoutMs = 3000,
): Promise<string> {
  const resp = await fetchFn(metricsUrl, { signal: AbortSignal.timeout(timeoutMs) })
  if (!resp.ok) throw new Error(`Metrics HTTP ${resp.status}`)
  return resp.text()
}
