import { readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import { writeTextFileAtomic } from '../atomic-data-write.ts'
import { BoundedCache } from '../bounded-cache.ts'
import { AUGMENT_STAGES, validChampionId, type AugmentStage, type StageResult, type StageSnapshot, type StageStatRow } from '../../shared/stage-statistics.ts'

const ORIGIN = 'https://data.aramkit.cn'
const MAX_BYTES = 2 * 1024 * 1024
const FRESH_MS = 6 * 60 * 60 * 1000
const RETAIN_MS = 7 * 24 * 60 * 60 * 1000
const CAPACITY = 8
type JsonObject = Record<string, unknown>
type Version = { version: string; dataPath: string; dataDate: string }
const object = (value: unknown): JsonObject => value && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {}
const patchValid = (value: unknown): value is string => typeof value === 'string' && /^\d{2}\.\d{1,2}$/.test(value)
const dateValid = (value: unknown): value is string => typeof value === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(value) && Number.isFinite(Date.parse(value))
function versionRow(value: unknown): Version {
  const row = object(value)
  if (!patchValid(row.version) || !dateValid(row.dataDate) || typeof row.dataPath !== 'string'
    || !/^data\/\d{2}\.\d{1,2}-\d{8}-[a-f0-9]{12}$/.test(row.dataPath)
    || !row.dataPath.startsWith(`data/${row.version}-`)) throw new Error('Invalid version')
  return { version: row.version, dataPath: row.dataPath, dataDate: row.dataDate }
}
export function parseStageVersion(value: unknown): Version {
  const manifest = object(value)
  if (!Array.isArray(manifest.versions) || manifest.versions.length > 100 || !patchValid(manifest.latest)) throw new Error('Invalid versions')
  return versionRow(manifest.versions.find(row => object(row).version === manifest.latest))
}
function rows(value: unknown, cached = false): StageStatRow[] {
  if (!Array.isArray(value) || value.length > 512) throw new Error('Invalid rows')
  const seen = new Set<number>()
  return value.map(input => {
    const row = object(input), id = row.id, samples = cached ? row.samples : row.sampleCount
    if (!Number.isSafeInteger(id) || Number(id) <= 0 || Number(id) > 100000 || seen.has(Number(id))
      || !Number.isSafeInteger(samples) || Number(samples) < 0
      || typeof row.winRate !== 'number' || !Number.isFinite(row.winRate) || row.winRate < 0 || row.winRate > 1
      || typeof row.pickRate !== 'number' || !Number.isFinite(row.pickRate) || row.pickRate < 0 || row.pickRate > 1) throw new Error('Invalid statistics')
    seen.add(Number(id))
    return { id: Number(id), winRate: row.winRate, pickRate: row.pickRate, samples: Number(samples) }
  })
}
export function parseStageDetail(value: unknown, championId: number, version: Version, now: number): StageSnapshot {
  const detail = object(value), augments = object(detail.augments), stages = object(augments.stages)
  if (!validChampionId(championId) || object(detail.champion).id !== championId) throw new Error('Wrong champion')
  const parsed = {} as Record<AugmentStage, StageStatRow[]>
  for (const stage of AUGMENT_STAGES) parsed[stage] = rows(stage === 'all' ? augments.all : stages[stage] ?? [])
  if (!parsed.all.length) throw new Error('No statistics')
  return { schema: 1, championId, source: 'ARAMKit', region: 'unknown', patch: version.version,
    dataDate: version.dataDate, dataPath: version.dataPath, fetchedAt: now, rows: parsed }
}
function parseCached(value: unknown, now: number): StageSnapshot {
  const row = object(value)
  if (row.schema !== 1 || row.source !== 'ARAMKit' || row.region !== 'unknown' || !validChampionId(row.championId)
    || typeof row.fetchedAt !== 'number' || !Number.isFinite(row.fetchedAt) || row.fetchedAt > now || now - row.fetchedAt > RETAIN_MS) throw new Error('Invalid cache')
  const version = versionRow({ version: row.patch, dataPath: row.dataPath, dataDate: row.dataDate })
  const parsed = {} as Record<AugmentStage, StageStatRow[]>
  for (const stage of AUGMENT_STAGES) parsed[stage] = rows(object(row.rows)[stage], true)
  if (!parsed.all.length) throw new Error('Empty cache')
  return { schema: 1, championId: row.championId, source: 'ARAMKit', region: 'unknown', patch: version.version,
    dataDate: version.dataDate, dataPath: version.dataPath, fetchedAt: row.fetchedAt, rows: parsed }
}

/** No cookies, tokens, account IDs, redirects, remote-controlled hosts or retries. */
export async function fetchStageJson(resource: string): Promise<unknown> {
  if (resource !== 'data/versions.json' && !/^data\/\d{2}\.\d{1,2}-\d{8}-[a-f0-9]{12}\/stats\/all\/champion-details\/[1-9]\d{0,4}\.json$/.test(resource)) throw new Error('Invalid resource')
  const controller = new AbortController(), timer = setTimeout(() => controller.abort(), 8000)
  try {
    const response = await fetch(`${ORIGIN}/${resource}`, { signal: controller.signal, redirect: 'error', credentials: 'omit', headers: { Accept: 'application/json' } })
    if (!response.ok || !response.body || Number(response.headers.get('content-length')) > MAX_BYTES) throw new Error('Unavailable')
    const chunks: Uint8Array[] = []; let length = 0
    for await (const chunk of response.body) {
      length += chunk.length
      if (length > MAX_BYTES) { controller.abort(); throw new Error('Oversized response') }
      chunks.push(chunk)
    }
    return JSON.parse(Buffer.concat(chunks).toString('utf8'))
  } finally { controller.abort(); clearTimeout(timer) }
}

export class StageStatisticsService {
  private cache = new BoundedCache<number, StageSnapshot>(CAPACITY)
  private loaded?: Promise<void>
  private pending?: { id: number; promise: Promise<StageResult> }
  private version?: { row: Version; at: number }
  private retryAt = 0
  private filename: string
  constructor(directory: string, private blocked: () => boolean, private request = fetchStageJson, private now = Date.now) {
    this.filename = path.join(directory, 'stage-statistics', 'aramkit.json')
  }
  private async load(): Promise<void> {
    this.loaded ??= (async () => {
      try {
        if ((await stat(this.filename)).size > MAX_BYTES) return
        const input = JSON.parse(await readFile(this.filename, 'utf8'))
        if (!Array.isArray(input) || input.length > CAPACITY) return
        for (const item of input) {
          try { const entry = parseCached(item, this.now()); this.cache.set(entry.championId, entry) } catch { /* Skip invalid entries. */ }
        }
      } catch { /* No valid local reference yet. */ }
    })()
    return this.loaded
  }
  private cached(id: number, issue?: StageResult['issue']): StageResult {
    const snapshot = this.cache.get(id)
    if (snapshot && this.now() - snapshot.fetchedAt <= RETAIN_MS) return { snapshot, cached: true, stale: this.now() - snapshot.fetchedAt >= FRESH_MS, issue }
    this.cache.delete(id)
    return { issue: issue || 'unavailable' }
  }
  async get(id: unknown, refresh: unknown = false): Promise<StageResult> {
    if (!validChampionId(id)) return { issue: 'invalid' }
    if (typeof refresh !== 'boolean') return { issue: 'invalid' }
    await this.load()
    const previous = this.cached(id)
    if (previous.snapshot && (!previous.stale || !refresh)) return previous
    if (this.blocked()) return this.cached(id, 'game-active')
    if (this.pending) return this.pending.id === id ? this.pending.promise : this.cached(id, 'busy')
    if (this.now() < this.retryAt) return this.cached(id, 'busy')
    const promise = this.refresh(id).finally(() => { this.pending = undefined })
    this.pending = { id, promise }
    return promise
  }
  private async refresh(id: number): Promise<StageResult> {
    this.retryAt = this.now() + 30000
    try {
      if (!this.version || this.now() - this.version.at >= FRESH_MS) {
        const row = parseStageVersion(await this.request('data/versions.json'))
        this.version = { row, at: this.now() }
      }
      if (this.blocked()) return this.cached(id, 'game-active')
      const version = this.version.row
      const snapshot = parseStageDetail(await this.request(`${version.dataPath}/stats/all/champion-details/${id}.json`), id, version, this.now())
      const previous = this.cache.get(id)
      if (previous && (snapshot.patch.localeCompare(previous.patch, 'en', { numeric: true }) < 0
        || snapshot.patch === previous.patch && snapshot.dataDate < previous.dataDate)) throw new Error('Older snapshot')
      // A late response cannot start disk work after gameplay begins.
      if (this.blocked()) return this.cached(id, 'game-active')
      this.cache.set(id, snapshot)
      const entries = [...this.cache.values()].filter(entry => this.now() - entry.fetchedAt <= RETAIN_MS)
      try { await writeTextFileAtomic(this.filename, JSON.stringify(entries)) } catch { /* Memory reference remains usable. */ }
      return { snapshot, cached: false, stale: false }
    } catch {
      this.retryAt = this.now() + 5 * 60 * 1000
      return this.cached(id, 'unavailable')
    }
  }
}
