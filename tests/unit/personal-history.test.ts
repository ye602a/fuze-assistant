import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import axios from 'axios'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type LCUService from '../../src/main/services/lcu/lcu-service.ts'
import { HextechAramQueryService, setHistoryGamePhase } from '../../src/main/services/match-history/hextech-aram-query-service.ts'
import { PersonalHistoryCache } from '../../src/main/services/match-history/personal-history-cache.ts'
import { SgpMatchHistoryService } from '../../src/main/services/match-history/sgp-match-history-service.ts'
import { historyPlatformFromIssuer, resolveHistoryPlatform } from '../../src/main/services/match-history/platform-resolver.ts'
import { PostGameHistorySync } from '../../src/main/services/match-history/post-game-sync.ts'

vi.mock('axios', () => ({ default: { get: vi.fn() } }))
vi.mock('../../src/main/modules/logger.ts', () => ({ default: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }))
vi.mock('../../src/main/data-loader.ts', () => ({
  loadChampionRoster: vi.fn().mockResolvedValue([]), loadAugmentDetail: vi.fn().mockResolvedValue({}), loadItems: vi.fn().mockResolvedValue([]),
}))

const puuid = 'fixture-player-0001'
const owner = { puuid, platformId: 'HN1', playerName: 'Test player' }
const savedMatch = { gameId: 1, gameCreation: 1000, gameDuration: 900, gameVersion: '16.20', championId: 103,
  kills: 4, deaths: 2, assists: 8, result: 'win' as const, augments: [{ id: 18 }], items: [{ id: 1001 }] }
const game = (id = 1, mode = 'KIWI') => ({
  gameId: id, gameCreation: id * 1000, gameDuration: 900, gameVersion: '16.20', platformId: 'HN1', gameMode: mode,
  mapId: 12, queueId: mode === 'KIWI' ? 2400 : 450,
  participants: [{ participantId: 1, puuid, championId: 103, kills: 4, deaths: 2, assists: 8, win: true, item0: 1001, playerAugment1: 18 }],
})
const failure = (status: number) => ({ response: { status }, message: 'PRIVATE_TOKEN_AND_IDENTITY_MUST_NOT_ESCAPE' })
let directory: string
let cache: PersonalHistoryCache
beforeEach(async () => {
  setHistoryGamePhase('Lobby')
  directory = await mkdtemp(path.join(os.tmpdir(), 'fuze-history-test-'))
  cache = new PersonalHistoryCache(path.join(directory, 'history.json'))
  vi.mocked(axios.get).mockReset()
})
afterEach(async () => { vi.useRealTimers(); await rm(directory, { recursive: true, force: true }) })

function lcu() {
  return {
    getCurrentSummoner: vi.fn().mockResolvedValue({ ...owner, gameName: 'Test player' }),
    getCurrentPlatformId: vi.fn().mockResolvedValue('HN1'),
    getEntitlementsAccessToken: vi.fn().mockResolvedValue('private-token'),
    getConnectionStatus: vi.fn().mockReturnValue({ discovered: true, phase: 'Lobby' }),
    getGameflowPhase: vi.fn().mockResolvedValue('Lobby'),
    getReadOnlyJsonEndpoint: vi.fn().mockResolvedValue({ status: 200, data: { platformId: 'HN1', games: { games: [game()] } } }),
  }
}
const serviceFor = (client: ReturnType<typeof lcu>) => new HextechAramQueryService(client as unknown as LCUService, cache)

describe('automatic history routing', () => {
  it('falls through missing authorization to chat without reading credentials', async () => {
    const read = vi.fn().mockResolvedValueOnce({ status: 200, data: {} }).mockResolvedValueOnce({ status: 200, data: { platformId: 'nj100' } })
    expect(await resolveHistoryPlatform(read)).toBe('NJ100')
    expect(read).toHaveBeenCalledTimes(2)
  })
  it('supports Tencent issuers with and without k8s, but never arbitrary hosts', () => {
    expect(historyPlatformFromIssuer('http://hn1-k8s-bcs-internal.lol.qq.com:28088')).toBe('HN1')
    expect(historyPlatformFromIssuer('http://nj100-bcs-internal.lol.qq.com:28088')).toBe('NJ100')
    expect(historyPlatformFromIssuer('https://hn1.lol.qq.com.evil.example')).toBeNull()
    expect(historyPlatformFromIssuer('https://secret@hn1.lol.qq.com')).toBeNull()
  })
})

describe('personal history fallback', () => {
  it('coalesces concurrent requests, persists only own matches, and performs no per-match fetch', async () => {
    const client = lcu()
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [{ json: game() }] } })
    const service = serviceFor(client)
    const [a, b] = await Promise.all([service.queryCurrent(), service.queryCurrent()])
    expect(a).toEqual(b)
    expect(a.source).toBe('sgp')
    expect(axios.get).toHaveBeenCalledTimes(1)
    expect(client.getReadOnlyJsonEndpoint).not.toHaveBeenCalled()
    expect((await cache.get(owner))?.matches).toHaveLength(1)
    const content = await readFile(path.join(directory, 'history.json'), 'utf8')
    expect(content).not.toContain('private-token')
    expect(content).not.toContain('participants')
  })
  it('falls back on 503, filters mixed queues before pagination and reuses the native window', async () => {
    const client = lcu()
    client.getReadOnlyJsonEndpoint.mockResolvedValue({ status: 200, data: { platformId: 'HN1', games: { games: [game(1), game(2, 'ARAM'), game(3)] } } })
    vi.mocked(axios.get).mockRejectedValue(failure(503))
    const service = serviceFor(client)
    const first = await service.queryCurrent({ count: 1 })
    const second = await service.queryCurrent({ count: 1, startIndex: 1 })
    expect(first).toMatchObject({ source: 'lcu', limited: true, warning: 'service-unavailable', hasMore: true })
    expect(first.matches[0].gameId).toBe(3)
    expect(second.matches[0].gameId).toBe(1)
    expect(second.hasMore).toBe(false)
    expect(axios.get).toHaveBeenCalledTimes(1)
    expect(client.getReadOnlyJsonEndpoint).toHaveBeenCalledTimes(1)
  })
  it('retains cached records when both providers fail; errors never become an empty success', async () => {
    await cache.save(owner, [savedMatch], 100)
    const client = lcu()
    client.getReadOnlyJsonEndpoint.mockResolvedValue({ status: 503, data: {} } as never)
    vi.mocked(axios.get).mockRejectedValue(failure(503))
    const page = await serviceFor(client).queryCurrent()
    expect(page).toMatchObject({ source: 'cache', queriedAt: 100, warning: 'service-unavailable' })
    expect(page.matches).toHaveLength(1)
    expect(JSON.stringify(page)).not.toContain('PRIVATE')
  })
  it('keeps older saved Mayhem games when the native recent window has only other modes', async () => {
    await cache.save(owner, [savedMatch], 100)
    const client = lcu()
    client.getReadOnlyJsonEndpoint.mockResolvedValue({ status: 200, data: { platformId: 'HN1', games: { games: [game(2, 'ARAM')] } } })
    vi.mocked(axios.get).mockRejectedValue(failure(503))
    const page = await serviceFor(client).queryCurrent()
    expect(page).toMatchObject({ source: 'lcu', limited: true, returnedCount: 1 })
    expect(page.matches[0].gameId).toBe(1)
  })
  it('bootstraps a missing region from the verified current-account native response', async () => {
    const client = lcu()
    client.getCurrentSummoner.mockResolvedValue({ puuid, gameName: 'Test player' } as never)
    client.getCurrentPlatformId.mockResolvedValue(null as never)
    const page = await serviceFor(client).queryCurrent()
    expect(page).toMatchObject({ source: 'lcu', platformId: 'HN1', returnedCount: 1 })
    expect(page.warning).toBeUndefined()
    expect(axios.get).not.toHaveBeenCalled()
  })
  it('accepts valid empty results without deleting previously saved history', async () => {
    await cache.save(owner, [savedMatch], 100)
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [] } })
    const page = await serviceFor(lcu()).queryCurrent()
    expect(page).toMatchObject({ source: 'sgp', returnedCount: 0 })
    expect((await cache.get(owner))?.matches).toHaveLength(1)
  })
  it('does not use a malformed success as proof of no matches', async () => {
    vi.mocked(axios.get).mockResolvedValue({ data: { unexpected: [] } })
    const client = lcu()
    client.getReadOnlyJsonEndpoint.mockResolvedValue({ status: 200, data: {} } as never)
    await expect(serviceFor(client).queryCurrent()).rejects.toMatchObject({ issue: 'invalid-response' })
  })
  it('never displays another account cache while a client is logged in', async () => {
    await cache.save({ ...owner, puuid: 'other-player-0002' }, [savedMatch], 200)
    expect(await serviceFor(lcu()).getCachedCurrent()).toBeNull()
  })
  it('discards results if the account changes during the request', async () => {
    const client = lcu()
    client.getCurrentSummoner.mockResolvedValueOnce({ ...owner, gameName: 'Test player' })
      .mockResolvedValue({ ...owner, puuid: 'other-player-0002' })
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [{ json: game() }] } })
    await expect(serviceFor(client).queryCurrent()).rejects.toMatchObject({ issue: 'account-changed' })
    expect(await cache.get()).toBeNull()
  })
  it('shows explicitly offline records without any remote request', async () => {
    await cache.save(owner, [savedMatch], 100)
    const client = lcu()
    client.getCurrentSummoner.mockResolvedValue(null)
    client.getConnectionStatus.mockReturnValue({ discovered: false, phase: null } as never)
    expect(await serviceFor(client).getCachedCurrent()).toMatchObject({ source: 'cache', offline: true })
    expect(axios.get).not.toHaveBeenCalled()
  })
  it('does not fetch online during a game', async () => {
    await cache.save(owner, [savedMatch], 100)
    const client = lcu()
    client.getGameflowPhase.mockResolvedValue('InProgress')
    expect(await serviceFor(client).queryCurrent()).toMatchObject({ source: 'cache', warning: 'game-busy' })
    expect(axios.get).not.toHaveBeenCalled()
    expect(client.getReadOnlyJsonEndpoint).not.toHaveBeenCalled()
  })
  it('refreshes authorization once on 401 but does not loop on 403', async () => {
    const client = lcu()
    vi.mocked(axios.get).mockRejectedValueOnce(failure(401)).mockResolvedValueOnce({ data: { games: [] } })
    const sgp = new SgpMatchHistoryService(client as unknown as LCUService, true)
    expect(await sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).toEqual([])
    expect(client.getEntitlementsAccessToken).toHaveBeenCalledTimes(2)
    vi.mocked(axios.get).mockReset().mockRejectedValue(failure(403))
    await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'forbidden' })
    expect(axios.get).toHaveBeenCalledTimes(1)
  })
  it('does not start the fallback if champion selection begins during a remote request', async () => {
    const client = lcu()
    vi.mocked(axios.get).mockImplementation(async () => { setHistoryGamePhase('ChampSelect'); throw failure(503) })
    await expect(serviceFor(client).queryCurrent()).rejects.toMatchObject({ issue: 'game-busy' })
    expect(client.getReadOnlyJsonEndpoint).not.toHaveBeenCalled()
  })
  it('does not retry authorization if a game begins while the request returns 401', async () => {
    const client = lcu()
    vi.mocked(axios.get).mockImplementation(async () => { setHistoryGamePhase('GameStart'); throw failure(401) })
    await expect(serviceFor(client).queryCurrent()).rejects.toMatchObject({ issue: 'game-busy' })
    expect(axios.get).toHaveBeenCalledTimes(1)
    expect(client.getEntitlementsAccessToken).toHaveBeenCalledTimes(1)
    expect(client.getReadOnlyJsonEndpoint).not.toHaveBeenCalled()
  })
  it('rejects an all-malformed SGP games array instead of returning empty history', async () => {
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [{ metadata: {} }] } })
    const sgp = new SgpMatchHistoryService(lcu() as unknown as LCUService, true)
    await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'invalid-response' })
  })
  it('isolates account switching between regions even when the player ID stays the same', async () => {
    const client = lcu()
    client.getCurrentSummoner.mockResolvedValueOnce({ ...owner }).mockResolvedValue({ ...owner, platformId: 'HN10' })
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [{ json: game() }] } })
    await expect(serviceFor(client).queryCurrent()).rejects.toMatchObject({ issue: 'account-changed' })
    expect(await cache.get()).toBeNull()
  })
  it('a 429 cooldown prevents repeated network and token work', async () => {
    const client = lcu()
    vi.mocked(axios.get).mockRejectedValue(failure(429))
    const sgp = new SgpMatchHistoryService(client as unknown as LCUService, true)
    for (let i = 0; i < 3; i++) await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'rate-limited' })
    expect(axios.get).toHaveBeenCalledTimes(1)
    expect(client.getEntitlementsAccessToken).toHaveBeenCalledTimes(1)
  })
  it('honors a longer server retry interval without a timer or background retry', async () => {
    vi.useFakeTimers()
    vi.mocked(axios.get).mockRejectedValue({ response: { status: 429, headers: { 'retry-after': '120' } } })
    const sgp = new SgpMatchHistoryService(lcu() as unknown as LCUService, true)
    await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'rate-limited' })
    await vi.advanceTimersByTimeAsync(61_000)
    await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'rate-limited' })
    expect(axios.get).toHaveBeenCalledTimes(1)
    await vi.advanceTimersByTimeAsync(60_000)
    await expect(sgp.getHextechAramSummaries(puuid, 'HN1', 0, 10)).rejects.toMatchObject({ issue: 'rate-limited' })
    expect(axios.get).toHaveBeenCalledTimes(2)
    expect(vi.getTimerCount()).toBe(0)
  })
})

describe('bounded local storage', () => {
  it('bounds account rotation and repeated reloads without accumulating temporary files', async () => {
    const filename = path.join(directory, 'history.json')
    for (let i = 0; i < 12; i++) {
      await cache.save({ ...owner, puuid: `fixture-player-${i}` }, Array.from({ length: 600 }, (_, index) =>
        ({ ...savedMatch, gameId: index + 1, gameCreation: index + 1 })), i)
      cache = new PersonalHistoryCache(filename)
    }
    const stored = JSON.parse(await readFile(filename, 'utf8'))
    expect(stored.accounts).toHaveLength(5)
    expect(stored.accounts.every((account: any) => account.matches.length === 500)).toBe(true)
    expect((await cache.get())?.puuid).toBe('fixture-player-11')
    expect(await readdir(directory)).toEqual(['history.json'])
  })
  it('merges by match ID, isolates regions, strips labels and bounds history', async () => {
    await cache.save(owner, Array.from({ length: 520 }, (_, i) => ({ ...savedMatch, gameId: i + 1, gameCreation: i + 1 })), 100)
    await cache.save(owner, [{ ...savedMatch, gameId: 520, gameCreation: 520, kills: 99, championName: 'untrusted-label' }], 200)
    await cache.save({ ...owner, platformId: 'HN10' }, [savedMatch], 300)
    const current = await new PersonalHistoryCache(path.join(directory, 'history.json')).get(owner)
    expect(current?.matches).toHaveLength(500)
    expect(current?.matches[0].kills).toBe(99)
    expect(current?.matches[0].championName).toBeUndefined()
    expect((await cache.get({ ...owner, platformId: 'HN10' }))?.matches).toHaveLength(1)
  })
  it('preserves a corrupt cache and still returns live results with a save warning', async () => {
    await writeFile(path.join(directory, 'history.json'), '{broken')
    vi.mocked(axios.get).mockResolvedValue({ data: { games: [{ json: game() }] } })
    const page = await serviceFor(lcu()).queryCurrent()
    expect(page).toMatchObject({ source: 'sgp', warning: 'cache-write-failed', returnedCount: 1 })
    expect(await readFile(path.join(directory, 'history.json'), 'utf8')).toBe('{broken')
  })
})

describe('post-game incremental capture', () => {
  it('leaves no timers across one hundred games and rejects late callbacks after stop', async () => {
    vi.useFakeTimers()
    const refresh = vi.fn().mockResolvedValue(undefined)
    const sync = new PostGameHistorySync(refresh)
    for (let i = 0; i < 100; i++) {
      sync.onPhase('InProgress'); sync.onPhase('EndOfGame'); sync.onPhase('Lobby')
      await vi.advanceTimersByTimeAsync(10_000)
      expect(vi.getTimerCount()).toBe(0)
    }
    expect(refresh).toHaveBeenCalledTimes(100)
    sync.stop()
    sync.onPhase('InProgress'); sync.onPhase('EndOfGame')
    await vi.advanceTimersByTimeAsync(20_000)
    expect(refresh).toHaveBeenCalledTimes(100)
    expect(vi.getTimerCount()).toBe(0)
  })
  it('runs once after a game, coalesces result/lobby changes and never repeats while idle', async () => {
    vi.useFakeTimers()
    const refresh = vi.fn().mockResolvedValue(undefined)
    const sync = new PostGameHistorySync(refresh)
    sync.onPhase('Lobby')
    await vi.advanceTimersByTimeAsync(20_000)
    expect(refresh).not.toHaveBeenCalled()
    sync.onPhase('InProgress'); sync.onPhase('EndOfGame'); sync.onPhase('Lobby')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(refresh).toHaveBeenCalledTimes(1)
    sync.onPhase('None')
    await vi.advanceTimersByTimeAsync(60_000)
    expect(refresh).toHaveBeenCalledTimes(1)
    sync.stop()
  })
  it('cancels a pending refresh when a new champion selection or disconnect occurs', async () => {
    vi.useFakeTimers()
    const refresh = vi.fn()
    const sync = new PostGameHistorySync(refresh)
    for (const phase of ['ChampSelect', null]) {
      sync.onPhase('InProgress'); sync.onPhase('EndOfGame'); sync.onPhase(phase)
      await vi.advanceTimersByTimeAsync(20_000)
    }
    expect(refresh).not.toHaveBeenCalled()
  })
})
