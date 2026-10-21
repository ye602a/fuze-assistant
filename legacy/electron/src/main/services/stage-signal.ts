import https from 'node:https'
import { STAGE_FIELD_NAMES, type ProbeStatus, type StageFieldShape, type StageSignalReport, type StageSignalResult } from '../../shared/stage-signal.ts'

export const PROBE_PATHS = ['/swagger/v3/openapi.json', '/liveclientdata/activeplayer', '/liveclientdata/eventdata'] as const
type ProbePath = typeof PROBE_PATHS[number]
export interface ProbeResponse { status: ProbeStatus; httpStatus?: number; data?: unknown }
type Reader = (path: ProbePath) => Promise<ProbeResponse>
const isRecord = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value)

/** Fixed loopback, GET only; no auth, redirects, environment proxy or response logging. */
export const readStageProbe: Reader = path => new Promise(resolve => {
  if (!PROBE_PATHS.includes(path)) { resolve({ status: 'unavailable' }); return }
  let done = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let body = ''
  const finish = (result: ProbeResponse) => {
    if (done) return
    done = true; clearTimeout(timer); body = ''; resolve(result)
  }
  const request = https.get({ hostname: '127.0.0.1', port: 2999, path, method: 'GET',
    rejectUnauthorized: false, agent: false }, response => {
    if (response.statusCode !== 200) {
      finish({ status: 'http-error', httpStatus: response.statusCode }); response.destroy(); return
    }
    const limit = path === PROBE_PATHS[0] ? 2 * 1024 * 1024 : 256 * 1024
    let bytes = 0
    response.setEncoding('utf8')
    response.on('data', (chunk: string) => {
      if (done) return
      bytes += Buffer.byteLength(chunk)
      if (bytes > limit) { finish({ status: 'too-large' }); response.destroy(); return }
      body += chunk
    })
    response.on('end', () => {
      if (done) return
      try { const data: unknown = JSON.parse(body); finish({ status: 'ok', httpStatus: 200, data }) }
      catch { finish({ status: 'invalid-json', httpStatus: 200 }) }
    })
    response.on('error', () => finish({ status: 'unavailable' }))
    response.on('aborted', () => finish({ status: 'unavailable' }))
  })
  timer = setTimeout(() => { finish({ status: 'timeout' }); request.destroy() }, 1500)
  timer.unref()
  request.on('error', () => finish({ status: 'unavailable' }))
})

/** Bounded inspection; unrecognized keys and all string values are discarded. */
export function summarizeStageProbe(responses: ProbeResponse[], checkedAt: number): StageSignalReport {
  const report: StageSignalReport = { checkedAt, actualStage: null, endpoints: [], schemaFields: [],
    playerFields: [], level: null, augmentEventCount: 0, truncated: false }
  for (const [index, endpoint] of (['schema', 'player', 'events'] as const).entries()) {
    const response = responses[index] || { status: 'unavailable' }
    report.endpoints.push({ endpoint, status: response.status, ...(response.httpStatus ? { httpStatus: response.httpStatus } : {}) })
  }
  const schema = responses[0]?.status === 'ok' ? responses[0].data : null
  const player = responses[1]?.status === 'ok' ? responses[1].data : null
  const events = responses[2]?.status === 'ok' ? responses[2].data : null
  // Inspect declared schema properties only; examples/descriptions may contain arbitrary values.
  let budget = 12000
  const walkSchema = (value: unknown, depth = 0) => {
    if (--budget < 0 || depth > 24) { report.truncated = true; return }
    if (!isRecord(value) && !Array.isArray(value)) return
    if (isRecord(value) && isRecord(value.properties)) {
      for (const name of STAGE_FIELD_NAMES) if (Object.hasOwn(value.properties, name) && !report.schemaFields.includes(name)) report.schemaFields.push(name)
    }
    for (const [key, child] of Object.entries(value)) {
      if (budget <= 0) { report.truncated = true; break }
      if (!['example', 'examples', 'description', 'default', 'enum'].includes(key)) walkSchema(child, depth + 1)
    }
  }
  if (isRecord(schema)) walkSchema(schema)
  // Only the active player's top-level candidate fields: nested arbitrary objects are not exported.
  if (isRecord(player)) {
    const level = player.level
    if (typeof level === 'number' && Number.isInteger(level) && level >= 1 && level <= 30) report.level = level
    for (const field of STAGE_FIELD_NAMES) {
      if (!Object.hasOwn(player, field)) continue
      const value = player[field]
      const kind = value === null ? 'null' : Array.isArray(value) ? 'array' : typeof value
      if (!['null', 'array', 'object', 'number', 'string', 'boolean'].includes(kind)) continue
      const shape: StageFieldShape = { field, kind: kind as StageFieldShape['kind'] }
      if (Array.isArray(value)) { shape.count = Math.min(value.length, 32); report.truncated ||= value.length > 32 }
      if (typeof value === 'number' && Number.isInteger(value) && value >= 0 && value <= 4) shape.smallInteger = value
      report.playerFields.push(shape)
    }
  }
  if (isRecord(events) && Array.isArray(events.Events)) {
    report.truncated ||= events.Events.length > 2048
    for (const event of events.Events.slice(0, 2048)) {
      // Counts are only leads; these names have not been validated on the CN client.
      if (isRecord(event) && typeof event.EventName === 'string'
        && ['AugmentSelected', 'AugmentChosen', 'AugmentGranted'].includes(event.EventName)) report.augmentEventCount++
    }
  }
  return report
}

/** Manual only. One in-flight inspection and a shared cooldown, no timers or disk cache. */
export class StageSignalProbe {
  private pending: Promise<StageSignalResult> | null = null
  private lastAttempt = -Infinity
  constructor(private readonly read: Reader = readStageProbe, private readonly now = Date.now,
    private readonly context: () => { revision: number; usable: boolean } = () => ({ revision: 0, usable: false })) {}
  inspect(): Promise<StageSignalResult> {
    if (this.pending) return this.pending
    const retryAfterMs = Math.max(0, 30000 - (this.now() - this.lastAttempt))
    if (retryAfterMs) return Promise.resolve({ report: null, retryAfterMs })
    this.lastAttempt = this.now()
    this.pending = this.run().finally(() => { this.pending = null })
    return this.pending
  }
  private async run(): Promise<StageSignalResult> {
    const before = this.context()
    const responses: ProbeResponse[] = []
    for (const path of PROBE_PATHS) {
      try { responses.push(await this.read(path)) } catch { responses.push({ status: 'unavailable' }) }
    }
    const after = this.context()
    const report = summarizeStageProbe(responses, this.now())
    report.context = { revision: after.revision, status: before.revision !== after.revision ? 'changed' : before.usable && after.usable ? 'stable' : 'unavailable' }
    // A report spanning a disconnect or phase change must not retain a previous session's clues.
    if (report.context.status === 'changed') { report.playerFields = []; report.schemaFields = []; report.level = null; report.augmentEventCount = 0 }
    return { report, retryAfterMs: Math.max(0, 30000 - (this.now() - this.lastAttempt)) }
  }
}
