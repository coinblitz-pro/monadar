import { connect, type Socket } from 'net'
import { c } from './colors'
import { bell } from './config'
import { logLine } from './log'
import { createDecoder, socketPath } from './protocol'
import { applyFrame, paint } from './render'
import { setupInput } from './ui'

function hint(path: string): string {
  const start = process.platform === 'darwin'
    ? 'sudo launchctl load /Library/LaunchDaemons/com.monadar.plist'
    : 'sudo systemctl start monadar'
  return [
    c.yellow('daemon not running') + ` (no socket at ${path})`,
    '',
    'Start it with:',
    `  ${c.cyan(start)}`,
    '',
    `Install the service first with ${c.cyan('monadar install')},`,
    `or run standalone without a daemon: ${c.cyan('monadar --foreground')}`,
  ].join('\n') + '\n'
}

// Attach viewer: connects to the daemon socket and renders incoming frames.
// Overlays and keypresses are handled locally. Reconnects if the daemon
// restarts; exits with a hint if no daemon is running at startup.
export function attach() {
  const path = socketPath()
  let everConnected = false
  let backoff = 500

  function shutdown() {
    if (process.stdin.isTTY) process.stdin.setRawMode(false)
    process.stdout.write('\x1b[?25h\n')
    process.exit()
  }
  process.on('SIGINT', shutdown)
  process.on('SIGTERM', shutdown)
  setupInput(paint, shutdown)

  process.stdout.write('\x1b[2J\x1b[H\x1b[?25l')

  function open() {
    const decode = createDecoder()
    const sock: Socket = connect(path)
    sock.setEncoding('utf8')

    sock.on('connect', () => {
      everConnected = true
      backoff = 500
    })

    sock.on('data', (chunk: string) => {
      for (const msg of decode(chunk)) {
        if (msg.type === 'hello') {
          for (const l of msg.backlog) logLine(l)
          if (msg.frame) applyFrame(msg.frame)
          paint()
        } else if (msg.type === 'snapshot') {
          applyFrame(msg.frame)
          if (msg.frame.logLine) logLine(msg.frame.logLine)
          if (bell && msg.frame.newCrit) process.stdout.write('\x07')
          paint()
        }
      }
    })

    sock.on('error', () => {})
    sock.on('close', () => {
      if (!everConnected) {
        // Never reached the daemon — it isn't running. Restore terminal + hint.
        if (process.stdin.isTTY) process.stdin.setRawMode(false)
        process.stdout.write('\x1b[?25h\x1b[2J\x1b[H')
        process.stdout.write(hint(path))
        process.exit(1)
      }
      process.stdout.write('\x1b[H' + c.dim(`reconnecting to ${path}…`) + '\x1b[K\x1b[J')
      setTimeout(open, backoff)
      backoff = Math.min(backoff * 2, 5000)
    })
  }

  open()
}
