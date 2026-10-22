import { spawn, type ChildProcess } from 'child_process'

const RESTART_DELAY_MS = 5_000

// Follows `journalctl -f` for a unit and calls onLine for every new message
// (without the trailing newline). Restarts journalctl if it exits.
// Needs root or the systemd-journal group.
export function followJournal(unit: string, onLine: (line: string) => void) {
  let child: ChildProcess | null = null
  let stopped = false

  function start() {
    let buf = ''
    child = spawn('journalctl', [ '-f', '-n', '0', '-o', 'cat', '-u', unit ], { stdio: [ 'ignore', 'pipe', 'ignore' ] })
    child.stdout!.setEncoding('utf-8')
    child.stdout!.on('data', (chunk: string) => {
      buf += chunk
      let nl: number
      while ((nl = buf.indexOf('\n')) !== -1) {
        onLine(buf.slice(0, nl))
        buf = buf.slice(nl + 1)
      }
    })
    child.on('error', () => {})
    child.on('exit', () => {
      child = null
      if (!stopped) setTimeout(start, RESTART_DELAY_MS).unref()
    })
  }

  start()
  process.on('exit', () => child?.kill())

  return {
    stop() {
      stopped = true
      child?.kill()
    },
  }
}
