import { existsSync } from 'fs'
import { dirname } from 'path'

const DEFAULT_SOCKET = '/run/monadar.sock'

let socketOverride: string | null = null

export function setSocketPath(p: string | null) {
  socketOverride = p
}

export function resolveSocketPath(
  override: string | null,
  env: NodeJS.ProcessEnv = process.env,
  exists: (path: string) => boolean = existsSync,
): string {
  if (override) return override
  const runDir = dirname(DEFAULT_SOCKET)
  if (exists(runDir)) return DEFAULT_SOCKET
  const base = env.XDG_RUNTIME_DIR || '/tmp'
  return `${base}/monadar.sock`
}

export function socketPath(): string {
  return resolveSocketPath(socketOverride)
}
