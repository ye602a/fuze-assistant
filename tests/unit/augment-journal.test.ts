import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AugmentJournal, JOURNAL_MAX_EVENTS, type JournalContext } from '../../src/main/services/match-history/augment-journal.ts'
import { readJournalContext } from '../../src/main/services/match-history/journal-context.ts'
import type LCUService from '../../src/main/services/lcu/lcu-service.ts'
import { PersonalHistoryCache } from '../../src/main/services/match-history/personal-history-cache.ts'
import { HextechAramQueryService } from '../../src/main/services/match-history/hextech-aram-query-service.ts'

vi.mock('../../src/main/modules/logger.ts', () => ({ default: { debug: vi.fn(), info: vi.fn(), warn: vi.fn(), error: vi.fn() } }))
vi.mock('../../src/main/data-loader.ts', () => ({ loadChampionRoster: vi.fn().mockResolvedValue([]),
  loadAugmentDetail: vi.fn().mockResolvedValue({ 1: { name: 'Localized augment' } }), loadItems: vi.fn().mockResolvedValue([]) }))
const context: JournalContext = { puuid: 'fixture-player-0001', platformId: 'HN1', playerName: 'Test', gameId: 123 }
let directory: string
let filename: string
let journal: AugmentJournal
const cards = (...ids: Array<number | null>) => ids.map((id, detectedSlot) => ({ id, detectedSlot, missing: id === null }))
const begin = (owner = context) => journal.onPhase('InProgress', async () => owner)
beforeEach(async () => {
  directory = await mkdtemp(path.join(os.tmpdir(), 'fuze-journal-test-'))
  filename = path.join(directory, 'journal.json')
  journal = new AugmentJournal(filename)
})
afterEach(async () => { await journal.stop().catch(() => {}); vi.useRealTimers(); await rm(directory, { recursive: true, force: true }) })

describe('accepted observation journal', () => {
  it('does no work for inactive/manual observations or empty sessions', async () => {
    journal.observe(cards(1, 2, 3), 3)
    await begin()
    await journal.onPhase('EndOfGame', async () => null)
    await expect(readFile(filename)).rejects.toMatchObject({ code: 'ENOENT' })
  })
  it('keeps the three slots, including a missing middle card and stale/unknown level', async () => {
    await begin()
    journal.observe([{ id: 3, detectedSlot: 2 }, { id: 1, detectedSlot: 0 }], null, true)
    const event = (await journal.read(context))!.sessions[0].events[0]
    expect(event).toMatchObject({ cards: [{ id: 1 }, null, { id: 3 }], level: null, partial: true })
  })
  it('deduplicates identical frames without timers or repeated persistence work', async () => {
    vi.useFakeTimers()
    await begin()
    journal.observe(cards(1, 2, 3), 3)
    expect(vi.getTimerCount()).toBe(1)
    for (let i = 0; i < 10000; i++) journal.observe(cards(1, 2, 3), 3)
    expect(vi.getTimerCount()).toBe(1)
    await journal.flush()
    expect(vi.getTimerCount()).toBe(0)
    journal.observe(cards(1, 2, 3), 3)
    expect(vi.getTimerCount()).toBe(0)
    expect((await journal.read(context))!.sessions[0].events).toHaveLength(1)
  })
  it('records changes and reappearance without guessing reroll count or chosen card', async () => {
    await begin()
    journal.observe(cards(1, 2, 3), 3)
    journal.observe(cards(1, null, 3), 3, true)
    journal.observe(cards(1, 4, 3), 3)
    journal.hide()
    journal.observe(cards(1, 4, 3), 7)
    const record = (await journal.read(context))!.sessions[0]
    expect(record.events.map(event => event.kind)).toEqual(['first', 'changed', 'changed', 'reappeared'])
    expect(record).not.toHaveProperty('selectedId')
    expect(record).not.toHaveProperty('round')
    expect(record.events[3].level).toBe(7)
  })
  it('bounds noisy input and marks the recording as capped', async () => {
    await begin()
    for (let id = 1; id <= JOURNAL_MAX_EVENTS + 20; id++) journal.observe(cards(id, 2, 3), 7)
    expect((await journal.read(context))!.sessions[0]).toMatchObject({ capped: true })
    expect((await journal.read(context))!.sessions[0].events).toHaveLength(JOURNAL_MAX_EVENTS)
  })
  it('persists only IDs and safe metadata, reloads and isolates account plus region', async () => {
    await begin()
    journal.observe([{ id: 1, name: 'private-label', token: 'do-not-persist' } as never], 3)
    await journal.onPhase('EndOfGame', async () => null)
    const saved = await readFile(filename, 'utf8')
    expect(saved).not.toContain('private-label')
    expect(saved).not.toContain('do-not-persist')
    const fresh = new AugmentJournal(filename)
    expect((await fresh.read(context))!.sessions).toHaveLength(1)
    expect((await fresh.read({ ...context, platformId: 'HN10' }))!.sessions).toHaveLength(0)
    expect((await fresh.read({ ...context, puuid: 'other-player-0002' }))!.sessions).toHaveLength(0)
    expect((await fresh.read(context))!.sessions[0].interrupted).toBe(false)
  })
  it('does not activate a late context after leaving the game', async () => {
    let finish!: (value: JournalContext) => void
    const starting = journal.onPhase('InProgress', () => new Promise(resolve => { finish = resolve }))
    await journal.onPhase('ChampSelect', async () => null)
    finish(context)
    await starting
    journal.observe(cards(1, 2, 3), 3)
    expect(await journal.read()).toBeNull()
  })
  it('marks a reconnect incomplete and resumes the same game without combining different games', async () => {
    await begin()
    journal.observe(cards(1, 2, 3), 3)
    await journal.onPhase(null, async () => null)
    await begin()
    journal.observe(cards(4, 5, 6), 7)
    await journal.onPhase('EndOfGame', async () => null)
    await begin({ ...context, gameId: 456 })
    journal.observe(cards(7, 8, 9), 3)
    const saved = (await journal.read(context))!.sessions
    expect(saved.map(record => record.gameId)).toEqual([456, 123])
    expect(saved[1].interrupted).toBe(true)
    expect(saved[1].events).toHaveLength(2)
  })
  it('treats a previous unclosed process as incomplete', async () => {
    await begin(); journal.observe(cards(1, 2, 3), 3); await journal.flush()
    expect((await new AugmentJournal(filename).read(context))!.sessions[0].interrupted).toBe(true)
  })
  it('preserves a corrupt file instead of replacing it with a fresh journal', async () => {
    await writeFile(filename, '{corrupt')
    await expect(begin()).rejects.toMatchObject({ issue: 'cache-unavailable' })
    journal.observe(cards(1, 2, 3), 3); await journal.flush()
    expect(await readFile(filename, 'utf8')).toBe('{corrupt')
  })
  it('bounds stored sessions and only returns the latest twenty for the account', async () => {
    for (let gameId = 1; gameId <= 102; gameId++) {
      await begin({ ...context, gameId }); journal.observe(cards(1, 2, 3), 3)
      await journal.onPhase('EndOfGame', async () => null)
    }
    const stored = JSON.parse(await readFile(filename, 'utf8'))
    expect(stored.sessions).toHaveLength(100)
    expect(stored.sessions.at(-1).gameId).toBe(3)
    const result = (await journal.read(context))!
    expect(result.sessions).toHaveLength(20)
    result.sessions[0].events[0].cards[0] = null
    expect((await journal.read(context))!.sessions[0].events[0].cards[0]).toEqual({ id: 1 })
  })
  it('preserves the last saved file on a failed write and can recover without retry timers', async () => {
    await begin(); journal.observe(cards(1, 2, 3), 3); await journal.flush()
    const previous = await readFile(filename, 'utf8')
    await mkdir(filename + '.tmp')
    journal.observe(cards(4, 2, 3), 3)
    await expect(journal.flush()).rejects.toMatchObject({ issue: 'cache-write-failed' })
    expect(await readFile(filename, 'utf8')).toBe(previous)
    await expect(journal.read(context)).rejects.toMatchObject({ issue: 'cache-write-failed' })
    await rm(filename + '.tmp', { recursive: true })
    await journal.flush()
    expect((await journal.read(context))!.sessions[0].events).toHaveLength(2)
  })
  it('does not repeat context requests on unchanged phase heartbeats', async () => {
    const read = vi.fn().mockResolvedValue(context)
    await journal.onPhase('InProgress', read)
    for (let i = 0; i < 20; i++) await journal.onPhase('InProgress', read)
    expect(read).toHaveBeenCalledOnce()
  })
  it('cannot restart from a delayed phase callback after shutdown', async () => {
    await begin(); journal.observe(cards(1, 2, 3), 3)
    await journal.stop()
    const read = vi.fn().mockResolvedValue(context)
    await journal.onPhase('InProgress', read)
    journal.observe(cards(4, 5, 6), 3)
    expect(read).not.toHaveBeenCalled()
    expect((await journal.read(context))!.sessions[0].events).toHaveLength(1)
  })
})

describe('session routing', () => {
  const lcu = (queue = 2400, gameId = 123, phase = 'InProgress') => ({
    getReadOnlyJsonEndpoint: vi.fn(async (endpoint: string) => ({ status: 200, data: endpoint.includes('gameflow')
      ? { phase, gameData: { gameId, queue: { id: queue } } }
      : { puuid: context.puuid, platformId: 'HN1', gameName: 'Test' } })),
    getCurrentPlatformId: vi.fn().mockResolvedValue('HN1'),
  })
  it('accepts a verified current Mayhem game without extra region reads', async () => {
    const client = lcu()
    expect(await readJournalContext(client as unknown as LCUService)).toEqual(context)
    expect(client.getCurrentPlatformId).not.toHaveBeenCalled()
  })
  it.each([[450, 123, 'InProgress'], [2400, 0, 'InProgress'], [2400, 123, 'EndOfGame']])(
    'rejects another mode, missing ID or non-live session %s/%s/%s', async (queue, id, phase) => {
      expect(await readJournalContext(lcu(queue as number, id as number, phase as string) as unknown as LCUService)).toBeNull()
    })
  it('rejects non-200 payloads even if they resemble a valid session', async () => {
    const client = lcu()
    client.getReadOnlyJsonEndpoint.mockResolvedValue({ status: 401, data: {} } as never)
    expect(await readJournalContext(client as unknown as LCUService)).toBeNull()
  })
})

describe('local journal query', () => {
  it('loads offline even without online match history, then joins final IDs only for the same owner and game', async () => {
    await begin(); journal.observe(cards(1, 2, 3), 3); await journal.onPhase('EndOfGame', async () => null)
    const client = { getCurrentSummoner: vi.fn().mockResolvedValue(null), getConnectionStatus: vi.fn().mockReturnValue({ discovered: false }) }
    const cache = new PersonalHistoryCache(path.join(directory, 'history.json'))
    const service = new HextechAramQueryService(client as unknown as LCUService, cache, () => journal)
    const emptyResult = await service.getSelectionJournal()
    expect(emptyResult).toMatchObject({ offline: true, sessions: [{ gameId: 123 }] })
    expect(emptyResult?.sessions[0].events[0].cards[0]?.name).toBe('Localized augment')
    expect(emptyResult?.sessions[0].finalAugmentIds).toBeUndefined()
    await cache.save(context, [{ gameId: 123, gameCreation: 1000, gameDuration: 600, gameVersion: 'test', championId: 1,
      kills: 1, deaths: 1, assists: 1, result: 'win', augments: [{ id: 2 }], items: [] }], 1000)
    expect((await service.getSelectionJournal())?.sessions[0].finalAugmentIds).toEqual([2])
  })
  it('does not show the last saved account while login is unresolved', async () => {
    await begin(); journal.observe(cards(1, 2, 3), 3)
    const client = { getCurrentSummoner: vi.fn().mockResolvedValue(null), getConnectionStatus: vi.fn().mockReturnValue({ discovered: true }) }
    const service = new HextechAramQueryService(client as unknown as LCUService, new PersonalHistoryCache(path.join(directory, 'history.json')), () => journal)
    await expect(service.getSelectionJournal()).rejects.toMatchObject({ issue: 'login-required' })
  })
})
