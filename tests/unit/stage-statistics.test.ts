import { afterEach, describe, expect, it, vi } from 'vitest'
import { mkdtemp, readFile, rm, mkdir, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { StageStatisticsService, fetchStageJson, parseStageDetail, parseStageVersion } from '../../src/main/services/stage-statistics.ts'
import { visibleStageRows } from '../../src/shared/stage-statistics.ts'

const version = { version: '16.20', dataPath: 'data/16.20-20261011-abcdef123456', dataDate: '2026-10-11' }
const manifest = { latest: '16.20', versions: [version] }
const row = { id: 1068, winRate: .52, pickRate: .12, sampleCount: 400 }
const detail = (id = 1) => ({ champion: { id }, augments: { all: [row], stages: { 1: [{ ...row, winRate: .56 }], 2: [{ ...row, sampleCount: 20 }], 3: [], 4: [{ ...row, winRate: .48 }] } } })
const directories: string[] = []
async function directory() { const dir = await mkdtemp(path.join(os.tmpdir(), 'fuze-stage-test-')); directories.push(dir); return dir }
afterEach(async () => { vi.unstubAllGlobals(); await Promise.all(directories.splice(0).map(dir => rm(dir, { recursive: true, force: true }))) })

describe('stage dataset validation', () => {
  it('keeps champion, stage and source separate without guessing region or inventing missing stages', () => {
    const data = parseStageDetail(detail(), 1, parseStageVersion(manifest), 1000)
    expect(data.region).toBe('unknown')
    expect(data.rows.all[0].winRate).toBe(.52)
    expect(data.rows['1'][0].winRate).toBe(.56)
    expect(visibleStageRows(data, '2')).toEqual([])
    expect(visibleStageRows(data, '3')).toEqual([])
    expect(data.rows['2'][0].samples).toBe(20)
    expect(() => parseStageDetail(detail(2), 1, version, 1000)).toThrow()
  })
  it.each(['https://evil.example/x', 'data/../x', 'data/16.19-20261011-abcdef123456', 'data/16.20-20261011-abcdef123456?q=1'])('rejects unsafe or inconsistent version path %s', dataPath => {
    expect(() => parseStageVersion({ ...manifest, versions: [{ ...version, dataPath }] })).toThrow()
  })
  it.each([{ winRate: 52 }, { pickRate: -1 }, { sampleCount: null }, { sampleCount: 1.5 }, { winRate: '0.5' }, { id: '../1' }])('rejects malformed numeric fields %o', change => {
    const input = detail(); input.augments.all = [{ ...row, ...change }] as typeof input.augments.all
    expect(() => parseStageDetail(input, 1, version, 1000)).toThrow()
  })
  it('rejects duplicate or oversized rows', () => {
    const input = detail(); input.augments.all = [row, row]
    expect(() => parseStageDetail(input, 1, version, 1000)).toThrow()
    input.augments.all = Array(513).fill(row)
    expect(() => parseStageDetail(input, 1, version, 1000)).toThrow()
  })
})

describe('bounded, on-demand stage cache', () => {
  it('does nothing at construction, deduplicates concurrent requests and serves persisted cache offline', async () => {
    const dir = await directory(), request = vi.fn(async (p: string) => p.endsWith('versions.json') ? manifest : detail())
    const service = new StageStatisticsService(dir, () => false, request, () => 1000)
    expect(request).not.toHaveBeenCalled()
    const [a, b] = await Promise.all([service.get(1), service.get(1)])
    expect(a).toEqual(b); expect(a.snapshot?.rows['1'][0].winRate).toBe(.56)
    expect(request).toHaveBeenCalledTimes(2)
    await service.get(1, true); expect(request).toHaveBeenCalledTimes(2)
    const offline = new StageStatisticsService(dir, () => true, request, () => 2000)
    expect((await offline.get(1)).cached).toBe(true)
    expect(request).toHaveBeenCalledTimes(2)
    const stored = await readFile(path.join(dir, 'stage-statistics/aramkit.json'), 'utf8')
    expect(stored).not.toMatch(/account|puuid|token|password|summoner|sampleCount/)
  })
  it('shows stale cache immediately; a failed manual refresh preserves it with backoff', async () => {
    let now = 1000
    const request = vi.fn(async (p: string) => p.endsWith('versions.json') ? manifest : detail())
    const service = new StageStatisticsService(await directory(), () => false, request, () => now)
    await service.get(1); now += 7 * 60 * 60 * 1000
    expect((await service.get(1)).stale).toBe(true); expect(request).toHaveBeenCalledTimes(2)
    request.mockRejectedValue(new Error('offline'))
    const result = await service.get(1, true)
    expect(result.issue).toBe('unavailable'); expect(result.snapshot?.championId).toBe(1)
    await service.get(1, true); expect(request).toHaveBeenCalledTimes(3)
    now += 8 * 24 * 60 * 60 * 1000
    expect((await service.get(1)).snapshot).toBeUndefined()
  })
  it('never initiates network while playing, and stops between manifest and detail when game starts', async () => {
    let blocked = true
    const request = vi.fn(async () => { blocked = true; return manifest })
    const service = new StageStatisticsService(await directory(), () => blocked, request)
    expect((await service.get(1)).issue).toBe('game-active'); expect(request).not.toHaveBeenCalled()
    blocked = false
    expect((await service.get(1)).issue).toBe('game-active'); expect(request).toHaveBeenCalledTimes(1)
  })
  it('does not persist or show a response arriving after game start', async () => {
    let blocked = false
    const request = vi.fn(async (p: string) => { if (p.endsWith('versions.json')) return manifest; blocked = true; return detail() })
    const dir = await directory(), service = new StageStatisticsService(dir, () => blocked, request)
    expect((await service.get(1)).issue).toBe('game-active')
    await expect(readFile(path.join(dir, 'stage-statistics/aramkit.json'))).rejects.toThrow()
  })
  it('limits disk retention to eight heroes and reuses the version request', async () => {
    let now = 1000
    const dir = await directory(), request = vi.fn(async (p: string) => p.endsWith('versions.json') ? manifest : detail(Number(p.match(/(\d+)\.json$/)?.[1])))
    const service = new StageStatisticsService(dir, () => false, request, () => now)
    for (let id = 1; id <= 10; id++) { expect((await service.get(id)).snapshot?.championId).toBe(id); now += 31000 }
    const cache = JSON.parse(await readFile(path.join(dir, 'stage-statistics/aramkit.json'), 'utf8'))
    expect(cache).toHaveLength(8); expect(cache.map((x: { championId: number }) => x.championId)).toEqual([3, 4, 5, 6, 7, 8, 9, 10])
    expect(request.mock.calls.filter(([p]) => p.endsWith('versions.json'))).toHaveLength(1)
  })
  it('rejects arbitrary IPC inputs without reading or requesting data', async () => {
    const request = vi.fn(), service = new StageStatisticsService(await directory(), () => false, request)
    for (const id of ['1', 0, NaN, '../1', 10001, {}]) expect((await service.get(id)).issue).toBe('invalid')
    expect((await service.get(1, 'yes')).issue).toBe('invalid'); expect(request).not.toHaveBeenCalled()
  })
  it('does not start a second hero download while another is pending', async () => {
    let finish!: (value: unknown) => void
    const request = vi.fn(() => new Promise(resolve => { finish = resolve }))
    const service = new StageStatisticsService(await directory(), () => false, request)
    const first = service.get(1)
    await vi.waitFor(() => expect(request).toHaveBeenCalledTimes(1))
    expect((await service.get(2)).issue).toBe('busy')
    finish({}); await first
    expect(request).toHaveBeenCalledTimes(1)
  })
  it('retains the last valid reference when the provider rolls back the patch', async () => {
    let now = 1000, rollback = false
    const request = vi.fn(async (p: string) => p.endsWith('versions.json')
      ? rollback ? { latest: '16.19', versions: [{ version: '16.19', dataDate: '2026-10-08', dataPath: 'data/16.19-20261008-abcdef123456' }] } : manifest
      : detail())
    const service = new StageStatisticsService(await directory(), () => false, request, () => now)
    await service.get(1); now += 7 * 60 * 60 * 1000; rollback = true
    const result = await service.get(1, true)
    expect(result.issue).toBe('unavailable'); expect(result.snapshot?.patch).toBe('16.20')
  })
  it('ignores corrupt or future cache data rather than trusting disk provenance', async () => {
    const dir = await directory(); await mkdir(path.join(dir, 'stage-statistics'))
    const data = parseStageDetail(detail(), 1, version, 99999)
    await writeFile(path.join(dir, 'stage-statistics/aramkit.json'), JSON.stringify([data]))
    const service = new StageStatisticsService(dir, () => true, vi.fn(), () => 1000)
    expect((await service.get(1)).snapshot).toBeUndefined()
  })
})

describe('public transport privacy and bounds', () => {
  it('uses only the pinned HTTPS host, omits credentials and rejects redirects', async () => {
    const fetcher = vi.fn(async () => new Response(JSON.stringify(manifest)))
    vi.stubGlobal('fetch', fetcher)
    expect(await fetchStageJson('data/versions.json')).toEqual(manifest)
    expect(fetcher).toHaveBeenCalledWith('https://data.aramkit.cn/data/versions.json', expect.objectContaining({ redirect: 'error', credentials: 'omit', headers: { Accept: 'application/json' } }))
    await expect(fetchStageJson('https://evil.example')).rejects.toThrow()
    expect(fetcher).toHaveBeenCalledTimes(1)
  })
  it('rejects oversized announced or streamed bodies', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => new Response('{}', { headers: { 'content-length': '99999999' } })))
    await expect(fetchStageJson('data/versions.json')).rejects.toThrow()
    vi.stubGlobal('fetch', vi.fn(async () => new Response(' '.repeat(2 * 1024 * 1024 + 1))))
    await expect(fetchStageJson('data/versions.json')).rejects.toThrow()
  })
})
