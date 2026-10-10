import https from 'node:https'

// A fixed loopback-only, read-only official endpoint. No identities leave this function.
export function parseActivePlayerLevel(value: unknown): number | null {
  const level = (value as { level?: unknown })?.level
  return typeof level === 'number' && Number.isInteger(level) && level >= 1 && level <= 30 ? level : null
}

export function requestActivePlayerLevel(): Promise<number | null> {
  return new Promise(resolve => {
    let settled = false
    let deadline: ReturnType<typeof setTimeout> | undefined
    const finish = (level: number | null) => { if (!settled) { settled = true; clearTimeout(deadline); resolve(level) } }
    const request = https.get({ hostname: '127.0.0.1', port: 2999,
      path: '/liveclientdata/activeplayer', method: 'GET', rejectUnauthorized: false,
      agent: false, timeout: 900 }, response => {
      if (response.statusCode !== 200) { response.destroy(); finish(null); return }
      let body = ''; let bytes = 0
      response.on('data', chunk => {
        bytes += chunk.length
        if (bytes > 65536) { response.destroy(); finish(null); return }
        body += chunk.toString('utf8')
      })
      response.on('end', () => {
        try { finish(parseActivePlayerLevel(JSON.parse(body))) } catch { finish(null) }
        body = ''
      })
      response.on('error', () => finish(null))
      response.on('aborted', () => finish(null))
    })
    deadline = setTimeout(() => { request.destroy(); finish(null) }, 1200)
    deadline.unref()
    request.on('timeout', () => { request.destroy(); finish(null) })
    request.on('error', () => finish(null))
  })
}

export class LiveLevelReader {
  private pending: Promise<number | null> | null = null
  private lastAttempt = -Infinity
  private generation = 0
  value: { level: number; checkedAt: number } | null = null
  constructor(private readonly read = requestActivePlayerLevel) {}
  reset() { this.generation++; this.value = null; this.lastAttempt = -Infinity }
  async poll(now = Date.now()) {
    if (this.pending || now - this.lastAttempt < 3000) return this.value
    this.lastAttempt = now
    const generation = this.generation
    this.pending = this.read()
    try {
      const level = await this.pending
      if (generation === this.generation) this.value = level === null ? null : { level, checkedAt: now }
    } catch { if (generation === this.generation) this.value = null }
    finally { this.pending = null }
    return this.value
  }
}
