import type { Thresholds } from './types'

const VALUE_FLAGS = new Set([
  '--config',
  '--socket',
  '--peers-min',
  '--p99-max',
  '--lag-max',
  '--rpc-max',
  '--tc-max',
  '--cpu',
])

const OPTIONAL_VALUE_FLAGS = new Set([ '--log' ])
const COMMANDS = new Set([ 'install', 'uninstall', 'status', 'daemon' ])
const SERVICE_DROP = new Set([ 'install', 'uninstall', 'status', 'daemon', '--foreground', '--init', '--print' ])

export type CliArgs = {
  raw: string[]
  install: boolean
  uninstall: boolean
  status: boolean
  daemon: boolean
  foreground: boolean
  verbose: boolean
  bell: boolean
  shouldInit: boolean
  print: boolean
  configPath: string | null
  socket: string | null
  cpu: string | null
  logFile: string | null
  intervalSec: number
  thresholdOverrides: Partial<Thresholds>
  servicePassthrough: string[]
}

function isValueToken(token: string | undefined): token is string {
  return token !== undefined && !token.startsWith('-') && !COMMANDS.has(token)
}

function parseIntArg(value: string | null): number | null {
  if (value === null || !/^\d+$/.test(value)) return null
  const n = Number(value)
  return Number.isSafeInteger(n) ? n : null
}

function findValue(args: readonly string[], flag: string): string | null {
  const i = args.findIndex(a => a === flag)
  return i !== -1 && isValueToken(args[i + 1]) ? args[i + 1] : null
}

export function parseArgs(rawArgs: readonly string[]): CliArgs {
  const raw = [ ...rawArgs ]
  const consumed = new Set<number>()

  for (let i = 0; i < raw.length; i++) {
    const arg = raw[i]
    if (VALUE_FLAGS.has(arg)) {
      consumed.add(i)
      if (isValueToken(raw[i + 1])) consumed.add(i + 1)
    } else if (OPTIONAL_VALUE_FLAGS.has(arg)) {
      consumed.add(i)
      if (isValueToken(raw[i + 1])) consumed.add(i + 1)
    } else if (arg.startsWith('-') || COMMANDS.has(arg)) {
      consumed.add(i)
    }
  }

  const intervalSec = parseIntArg(raw.find((a, i) => !consumed.has(i) && /^\d+$/.test(a)) ?? null) ?? 1

  const logIdx = raw.findIndex(a => a === '--log')
  const logFile = logIdx === -1
    ? null
    : isValueToken(raw[logIdx + 1])
      ? raw[logIdx + 1]
      : 'monadar.log'

  const thresholdOverrides: Partial<Thresholds> = {}
  const pairs: [keyof Thresholds, string][] = [
    [ 'peers_min', '--peers-min' ],
    [ 'p99_max', '--p99-max' ],
    [ 'lag_max', '--lag-max' ],
    [ 'rpc_max', '--rpc-max' ],
    [ 'tc_max', '--tc-max' ],
  ]
  for (const [ key, flag ] of pairs) {
    const n = parseIntArg(findValue(raw, flag))
    if (n !== null) thresholdOverrides[key] = n
  }

  return {
    raw,
    install: raw.includes('install'),
    uninstall: raw.includes('uninstall'),
    status: raw.includes('status'),
    daemon: raw.includes('daemon'),
    foreground: raw.includes('--foreground'),
    verbose: raw.includes('--verbose') || raw.includes('-v'),
    bell: raw.includes('--bell'),
    shouldInit: raw.includes('--init'),
    print: raw.includes('--print'),
    configPath: findValue(raw, '--config'),
    socket: findValue(raw, '--socket'),
    cpu: findValue(raw, '--cpu'),
    logFile,
    intervalSec,
    thresholdOverrides,
    servicePassthrough: raw.filter(a => !SERVICE_DROP.has(a)),
  }
}
