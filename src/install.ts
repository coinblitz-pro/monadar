import { execFileSync } from 'child_process'
import { existsSync, unlinkSync, writeFileSync } from 'fs'
import { connect } from 'net'
import { resolve } from 'path'
import { c } from './colors'
import { cliArgs } from './config'
import { socketPath } from './socket'

const SYSTEMD_PATH = '/etc/systemd/system/monadar.service'
const LAUNCHD_LABEL = 'com.monadar'
const LAUNCHD_PATH = `/Library/LaunchDaemons/${LAUNCHD_LABEL}.plist`

// Flags the user passed to `install` are forwarded to the daemon's ExecStart
// (e.g. --config, --socket, --cpu, --log, interval), so the service runs with
// the same configuration it was installed with.
function daemonArgs(): string[] {
  const passthrough = cliArgs.servicePassthrough
  return [ resolve(process.argv[1]), 'daemon', ...passthrough ]
}

export function quoteSystemdArg(value: string): string {
  const escapedSpecifiers = value.replace(/%/g, '%%').replace(/\$/g, () => '$$')
  if (/^[A-Za-z0-9_@%+=:,./-]+$/.test(escapedSpecifiers)) return escapedSpecifiers
  return `"${escapedSpecifiers
    .replace(/\\/g, '\\\\')
    .replace(/"/g, '\\"')
    .replace(/\n/g, '\\n')
    .replace(/\t/g, '\\t')}"`
}

function xmlEscape(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&apos;')
}

export function buildSystemdUnit(argv: string[]): string {
  const exec = argv.map(quoteSystemdArg).join(' ')
  return [
    '[Unit]',
    'Description=monadar node monitor',
    'After=network.target',
    '',
    '[Service]',
    'Type=simple',
    `ExecStart=${exec}`,
    'Restart=always',
    'RestartSec=2',
    '',
    '[Install]',
    'WantedBy=multi-user.target',
    '',
  ].join('\n')
}

export function buildLaunchdPlist(argv: string[]): string {
  const args = argv.map(a => `    <string>${xmlEscape(a)}</string>`).join('\n')
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">',
    '<plist version="1.0">',
    '<dict>',
    `  <key>Label</key><string>${LAUNCHD_LABEL}</string>`,
    '  <key>ProgramArguments</key>',
    '  <array>',
    args,
    '  </array>',
    '  <key>RunAtLoad</key><true/>',
    '  <key>KeepAlive</key><true/>',
    '  <key>StandardErrorPath</key><string>/var/log/monadar.err.log</string>',
    '  <key>StandardOutPath</key><string>/var/log/monadar.out.log</string>',
    '</dict>',
    '</plist>',
    '',
  ].join('\n')
}

export function install(opts: { print?: boolean }) {
  const isMac = process.platform === 'darwin'
  const argv = [ process.execPath, ...daemonArgs() ]
  const unit = isMac ? buildLaunchdPlist(argv) : buildSystemdUnit(argv)
  const path = isMac ? LAUNCHD_PATH : SYSTEMD_PATH

  if (opts.print) {
    process.stdout.write(unit)
    return
  }

  if (process.argv[1].endsWith('.ts')) {
    console.error(c.yellow('Warning:') + ' installing with a .ts entrypoint (tsx). Install the built dist/monadar.js for production.')
  }

  try {
    writeFileSync(path, unit)
  } catch (e) {
    console.error(c.red('Failed to write ') + path + ': ' + (e as Error).message)
    console.error('Run with sudo (system-level service).')
    process.exit(1)
  }

  console.log(`Wrote ${path}`)
  if (isMac) {
    console.log('Enable & start:')
    console.log(`  ${c.cyan(`sudo launchctl load -w ${path}`)}`)
  } else {
    try { execFileSync('systemctl', ['daemon-reload']) } catch {}
    console.log('Enable & start:')
    console.log(`  ${c.cyan('sudo systemctl enable --now monadar')}`)
  }
}

export function uninstall() {
  const isMac = process.platform === 'darwin'
  const path = isMac ? LAUNCHD_PATH : SYSTEMD_PATH
  if (isMac) {
    console.log('Stop & remove:')
    console.log(`  ${c.cyan(`sudo launchctl unload ${path}`)}`)
  } else {
    console.log('Stop & remove:')
    console.log(`  ${c.cyan('sudo systemctl disable --now monadar')}`)
  }
  try {
    if (existsSync(path)) { unlinkSync(path); console.log(`Removed ${path}`) }
    else console.log(`Not installed: ${path}`)
  } catch (e) {
    console.error(c.red('Failed to remove ') + path + ': ' + (e as Error).message)
    process.exit(1)
  }
  if (!isMac) { try { execFileSync('systemctl', ['daemon-reload']) } catch {} }
}

// Probe the socket: connect briefly and report whether a daemon is bound.
export function status() {
  const path = socketPath()
  const sock = connect(path)
  const done = (msg: string, code: number) => { sock.destroy(); console.log(msg); process.exit(code) }
  sock.setTimeout(2000)
  sock.on('connect', () => done(`${c.green('● running')} — daemon listening on ${path}`, 0))
  sock.on('timeout', () => done(`${c.yellow('● unknown')} — socket ${path} not responding`, 1))
  sock.on('error', () => done(`${c.red('● stopped')} — no daemon at ${path}`, 1))
}
