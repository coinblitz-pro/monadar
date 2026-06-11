import { frameEventsMessage, frameStatusMessage } from './telegram'
import type { Frame } from './types'

type BotOpts = {
  token: string
  chatId: string
  getFrame: () => Frame | null
}

const HELP = [
  'monadar bot commands:',
  '/status — current node status',
  '/events — recent events',
  '/help — this message',
].join('\n')

async function tgCall(token: string, method: string, body: object, timeoutMs: number): Promise<any> {
  const resp = await fetch(`https://api.telegram.org/bot${token}/${method}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  })
  return resp.json()
}

// Telegram bot via long polling (getUpdates) — no public endpoint needed, so it
// works behind a node's firewall. Only replies to the configured chat_id. Runs
// as an independent loop alongside the daemon's tick loop. Returns a stop fn.
export function startTelegramBot(opts: BotOpts): () => void {
  let stopped = false

  async function reply(text: string) {
    try {
      await tgCall(opts.token, 'sendMessage', { chat_id: opts.chatId, text, parse_mode: 'HTML' }, 5000)
    } catch {}
  }

  function handle(text: string) {
    // Strip a trailing @botname and any arguments; match the bare command.
    const cmd = text.trim().split(/\s+/)[0].split('@')[0].toLowerCase()
    const f = opts.getFrame()
    if (cmd === '/status') reply(f ? frameStatusMessage(f) : 'No data yet — daemon is still starting.')
    else if (cmd === '/events') reply(f ? frameEventsMessage(f) : 'No data yet — daemon is still starting.')
    else if (cmd === '/help' || cmd === '/start') reply(HELP)
  }

  async function loop() {
    let offset = 0
    // Drain any updates queued before startup so we don't reply to stale commands.
    try {
      const init = await tgCall(opts.token, 'getUpdates', { timeout: 0 }, 10000)
      if (init?.result?.length) offset = init.result[init.result.length - 1].update_id + 1
    } catch {}

    while (!stopped) {
      try {
        const res = await tgCall(opts.token, 'getUpdates', { offset, timeout: 50, allowed_updates: [ 'message' ] }, 55000)
        if (!res?.ok || !Array.isArray(res.result)) {
          await new Promise(r => setTimeout(r, 2000))
          continue
        }
        for (const u of res.result) {
          offset = u.update_id + 1
          const msg = u.message
          if (!msg?.text) continue
          if (String(msg.chat?.id) !== String(opts.chatId)) continue
          handle(msg.text)
        }
      } catch {
        // Network error or long-poll timeout — brief backoff, then retry.
        if (!stopped) await new Promise(r => setTimeout(r, 2000))
      }
    }
  }

  loop()
  return () => { stopped = true }
}
