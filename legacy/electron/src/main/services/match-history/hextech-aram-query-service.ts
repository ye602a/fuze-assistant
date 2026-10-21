import type {
  HextechAramMatchHistoryAsset,
  HextechAramMatchHistoryMatch,
  HextechAramMatchHistoryPage,
  HextechAramMatchHistoryQuery,
  MatchHistoryIssue,
  AugmentJournalResult,
} from '../../../shared/ipc-contract.ts'
import { loadAugmentDetail, loadChampionRoster, loadItems } from '../../data-loader.ts'
import logger from '../../modules/logger.ts'
import type LCUService from '../lcu/lcu-service.ts'
import { normalizeGame } from './local-match-history-service.ts'
import { isHextechAramGame } from './statistics.ts'
import { SgpMatchHistoryService } from './sgp-match-history-service.ts'
import { HistoryError, historyIssue, httpHistoryError } from './history-error.ts'
import { PersonalHistoryCache, validHistoryPuuid, type HistoryOwner, type PersonalHistory } from './personal-history-cache.ts'
import { historyPlatformFromPayload } from './platform-resolver.ts'
import type { StoredMatchHistoryGame } from './types.ts'
import { getAugmentJournal, type AugmentJournal } from './augment-journal.ts'

export const DEFAULT_HEXTECH_ARAM_QUERY_COUNT = 10
export const MAX_HEXTECH_ARAM_QUERY_COUNT = 20
export const MAX_HEXTECH_ARAM_QUERY_START_INDEX = 1_000
let observedHistoryPhase: string | null | undefined
export function setHistoryGamePhase(phase: string | null): void { observedHistoryPhase = phase }

type AnyRecord = Record<string, unknown>

type AssetLabel = {
  name?: string
  iconUrl?: string
}

export type HextechAramQueryLabels = {
  champions?: ReadonlyMap<number, AssetLabel>
  augments?: ReadonlyMap<number, AssetLabel>
  items?: ReadonlyMap<number, AssetLabel>
}

type BuildPageOptions = {
  games: StoredMatchHistoryGame[]
  currentPuuid: string
  playerName: string
  platformId: string
  startIndex: number
  count: number
  scannedCount: number
  queriedAt: number
  labels?: HextechAramQueryLabels
}

function isRecord(value: unknown): value is AnyRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function asString(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

function formatPlayerName(gameName: string, tagLine: string): string {
  if (!gameName) return ''
  return tagLine ? `${gameName}#${tagLine}` : gameName
}

function getPlayerName(currentSummoner: AnyRecord): string {
  return formatPlayerName(
    asString(currentSummoner.gameName) || asString(currentSummoner.displayName),
    asString(currentSummoner.tagLine),
  )
}

function toAsset(id: number, labels?: ReadonlyMap<number, AssetLabel>): HextechAramMatchHistoryAsset {
  const label = labels?.get(id)
  return {
    id,
    ...(label?.name ? { name: label.name } : {}),
    ...(label?.iconUrl ? { iconUrl: label.iconUrl } : {}),
  }
}

function toMatch(
  game: StoredMatchHistoryGame,
  currentPuuid: string,
  labels: HextechAramQueryLabels,
): HextechAramMatchHistoryMatch | null {
  if (!isHextechAramGame(game)) return null

  const participant = game.participants.find((entry) => entry.puuid === currentPuuid)
  if (!participant) return null

  const champion = labels.champions?.get(participant.championId)
  const isRemake = game.endOfGameResult.startsWith('Abort_') || participant.gameEndedInEarlySurrender
  return {
    gameId: game.gameId,
    gameCreation: game.gameCreation,
    gameDuration: game.gameDuration,
    gameVersion: game.gameVersion,
    championId: participant.championId,
    ...(champion?.name ? { championName: champion.name } : {}),
    ...(champion?.iconUrl ? { championIconUrl: champion.iconUrl } : {}),
    kills: participant.kills,
    deaths: participant.deaths,
    assists: participant.assists,
    result: isRemake ? 'remake' : participant.win ? 'win' : 'loss',
    augments: participant.augments.map((id) => toAsset(id, labels.augments)),
    items: participant.items.map((id) => toAsset(id, labels.items)),
  }
}

function getLocalizedItemName(item: AnyRecord): string {
  if (typeof item.name === 'string') return item.name
  if (!isRecord(item.name)) return ''
  return asString(item.name.zh_CN) || asString(item.name.zh_cn) || asString(item.name.en_us)
}

export function normalizeHextechAramQuery(
  query: HextechAramMatchHistoryQuery | null | undefined,
): Required<Pick<HextechAramMatchHistoryQuery, 'startIndex' | 'count'>> {
  const requestedStartIndex = Number(query?.startIndex)
  const requestedCount = Number(query?.count)
  const startIndex = Number.isInteger(requestedStartIndex) && requestedStartIndex >= 0
    ? Math.min(requestedStartIndex, MAX_HEXTECH_ARAM_QUERY_START_INDEX)
    : 0
  const count = Number.isInteger(requestedCount) && requestedCount > 0
    ? Math.min(requestedCount, MAX_HEXTECH_ARAM_QUERY_COUNT)
    : DEFAULT_HEXTECH_ARAM_QUERY_COUNT

  return { startIndex, count }
}

export function buildHextechAramMatchHistoryPage(
  options: BuildPageOptions,
): HextechAramMatchHistoryPage {
  const labels = options.labels || {}
  const seenGameIds = new Set<number>()
  const matches = [...options.games]
    .sort((left, right) => right.gameCreation - left.gameCreation)
    .flatMap((game) => {
      if (seenGameIds.has(game.gameId)) return []
      seenGameIds.add(game.gameId)
      const match = toMatch(game, options.currentPuuid, labels)
      return match ? [match] : []
    })

  return {
    playerName: options.playerName,
    platformId: options.platformId,
    queriedAt: options.queriedAt,
    startIndex: options.startIndex,
    count: options.count,
    returnedCount: matches.length,
    hasPrevious: options.startIndex > 0,
    hasMore: options.scannedCount >= options.count,
    matches,
  }
}

export class HextechAramQueryService {
  private readonly sgpMatchHistoryService: SgpMatchHistoryService
  private inFlight: { key: string; promise: Promise<HextechAramMatchHistoryPage> } | null = null
  private nativeSnapshot: { key: string; until: number; games: StoredMatchHistoryGame[] } | null = null

  constructor(private readonly lcuService: LCUService, private readonly cache = new PersonalHistoryCache(),
    private readonly journal: () => AugmentJournal = getAugmentJournal) {
    this.sgpMatchHistoryService = new SgpMatchHistoryService(lcuService, true)
  }

  private async owner(): Promise<HistoryOwner | null> {
    const summoner = await this.lcuService.getCurrentSummoner()
    if (!isRecord(summoner) || !validHistoryPuuid(summoner.puuid)) return null
    const platformId = historyPlatformFromPayload(summoner) || await this.lcuService.getCurrentPlatformId()
    return { puuid: summoner.puuid, platformId: platformId || '', playerName: getPlayerName(summoner) }
  }

  async getSelectionJournal(): Promise<AugmentJournalResult['data']> {
    const owner = await this.owner()
    if (owner && !owner.platformId) throw new HistoryError('platform-unknown')
    if (!owner && this.lcuService.getConnectionStatus().discovered) throw new HistoryError('login-required')
    const saved = await this.journal().read(owner || undefined)
    if (!saved) return { playerName: owner?.playerName || '', platformId: owner?.platformId || '', offline: !owner, sessions: [] }
    const [augmentDetails, history, roster] = await Promise.all([loadAugmentDetail().catch(() => ({})), this.cache.get(saved.owner).catch(() => null), loadChampionRoster().catch(() => [])])
    const championNames = new Map(roster.map((champion: AnyRecord) => [Number(champion.championId ?? champion.id), asString(champion.nameCN) || asString(champion.nameEN)]))
    const labels = new Map(Object.entries(augmentDetails).map(([id, value]) => {
      const augment = isRecord(value) ? value : {}
      return [Number(id), { name: asString(augment.name), iconUrl: asString(augment.iconUrl) }]
    }))
    if (owner) await this.assertOwner(owner)
    else if (this.lcuService.getConnectionStatus().discovered) throw new HistoryError('account-changed')
    const finalMatches = new Map(history?.matches.map(match => [match.gameId, match]))
    return { playerName: saved.owner.playerName, platformId: saved.owner.platformId, offline: !owner,
      sessions: saved.sessions.map(session => ({ ...session,
        championId: finalMatches.get(session.gameId)?.championId,
        championName: championNames.get(finalMatches.get(session.gameId)?.championId || 0),
        result: finalMatches.get(session.gameId)?.result,
        finalAugmentIds: finalMatches.get(session.gameId)?.augments.map(augment => augment.id),
        events: session.events.map(event => ({ ...event, cards: event.cards.map(card => card ? toAsset(card.id, labels) : null) })),
      })) }
  }

  async getCachedCurrent(query?: HextechAramMatchHistoryQuery): Promise<HextechAramMatchHistoryPage | null> {
    const owner = await this.owner()
    if (owner && !owner.platformId) throw new HistoryError('platform-unknown')
    if (!owner && this.lcuService.getConnectionStatus().discovered) throw new HistoryError('login-required')
    const cached = await this.cache.get(owner || undefined)
    const page = cached ? await this.cachedPage(cached, query, !owner) : null
    if (owner) await this.assertOwner(owner)
    else if (this.lcuService.getConnectionStatus().discovered) throw new HistoryError('account-changed')
    return page
  }

  private async cachedPage(cached: PersonalHistory, query?: HextechAramMatchHistoryQuery, offline = false,
    warning?: MatchHistoryIssue): Promise<HextechAramMatchHistoryPage> {
    const range = normalizeHextechAramQuery(query)
    const labels = await this.loadLabels()
    const championId = Number.isSafeInteger(query?.championId) && query!.championId! > 0 ? query!.championId : undefined
    const result = ['win', 'loss', 'remake'].includes(query?.result || '') ? query!.result : undefined
    const filtered = query?.reviewOnly === true ? cached.matches.filter(match => (!championId || match.championId === championId) && (!result || match.result === result)) : cached.matches
    const matches = filtered.slice(range.startIndex, range.startIndex + range.count).map((match) => ({
      ...match,
      championName: labels.champions?.get(match.championId)?.name,
      championIconUrl: labels.champions?.get(match.championId)?.iconUrl,
      augments: match.augments.map(({ id }) => toAsset(id, labels.augments)),
      items: match.items.map(({ id }) => toAsset(id, labels.items)),
    }))
    return { playerName: cached.playerName, platformId: cached.platformId, queriedAt: cached.updatedAt,
      ...range, ...(query?.reviewOnly === true ? { savedCount: cached.matches.length, matchedCount: filtered.length,
        reviewChampions: [...new Set(cached.matches.map(match => match.championId))].map(id => ({ id, name: labels.champions?.get(id)?.name })) } : {}), matches, returnedCount: matches.length, hasPrevious: range.startIndex > 0,
      hasMore: range.startIndex + range.count < filtered.length,
      source: 'cache', offline, limited: true, warning }
  }

  queryCurrent(query?: HextechAramMatchHistoryQuery): Promise<HextechAramMatchHistoryPage> {
    if (query?.reviewOnly === true) return this.getCachedCurrent(query).then(page => {
      if (!page) throw new HistoryError('cache-unavailable')
      return page
    })
    const range = normalizeHextechAramQuery(query)
    const key = `${range.startIndex}:${range.count}`
    if (this.inFlight) return this.inFlight.key === key ? this.inFlight.promise : Promise.reject(new HistoryError('rate-limited'))
    const promise = this.refresh(range).finally(() => { this.inFlight = null })
    this.inFlight = { key, promise }
    return promise
  }

  private async nativeGames(owner: HistoryOwner): Promise<StoredMatchHistoryGame[]> {
    const key = `${owner.platformId}:${owner.puuid}`
    if (this.nativeSnapshot?.key === key && this.nativeSnapshot.until > Date.now()) return this.nativeSnapshot.games
    if (!this.canFetch()) throw new HistoryError('game-busy')
    const response = await this.lcuService.getReadOnlyJsonEndpoint(
      '/lol-match-history/v1/products/lol/current-summoner/matches?begIndex=0&endIndex=49',
    )
    if (response?.status !== 200) throw httpHistoryError(response?.status)
    const payload = isRecord(response.data) ? response.data : {}
    const container = isRecord(payload.games) ? payload.games : {}
    if (!Array.isArray(container.games)) throw new HistoryError('invalid-response')
    // A current-account response can bootstrap routing without guessing another player's platform.
    const reported = historyPlatformFromPayload(payload)
    if (owner.platformId && reported && owner.platformId !== reported) throw new HistoryError('account-changed')
    if (!owner.platformId) owner.platformId = reported || ''
    let ownedRecords = 0
    const games = container.games.flatMap((game) => {
      const normalized = normalizeGame(game, owner.platformId, Date.now())
      if (!normalized || !normalized.participants.some((p) => p.puuid === owner.puuid)) return []
      ownedRecords += 1
      if (!owner.platformId) owner.platformId = historyPlatformFromPayload(normalized) || ''
      return normalized.platformId === owner.platformId && isHextechAramGame(normalized) ? [normalized] : []
    }).sort((a, b) => b.gameCreation - a.gameCreation)
    if (container.games.length && !ownedRecords) throw new HistoryError('invalid-response')
    this.nativeSnapshot = { key: `${owner.platformId}:${owner.puuid}`, until: Date.now() + 30_000, games }
    return games
  }

  private async refresh(query: Required<Pick<HextechAramMatchHistoryQuery, 'startIndex' | 'count'>>): Promise<HextechAramMatchHistoryPage> {
    const owner = await this.owner()
    if (!owner) {
      if (this.lcuService.getConnectionStatus().discovered) throw new HistoryError('login-required')
      const cached = await this.cache.get()
      if (cached) return this.cachedPage(cached, query, true, 'login-required')
      throw new HistoryError('login-required')
    }
    const phase = await this.lcuService.getGameflowPhase()
    if (!['None', 'Lobby', 'EndOfGame', 'PreEndOfGame', 'WaitingForStats'].includes(phase || '')) {
      const cached = owner.platformId ? await this.cache.get(owner) : null
      if (cached) return this.cachedPage(cached, query, false, 'game-busy')
      throw new HistoryError('game-busy')
    }
    let source: 'sgp' | 'lcu' = 'sgp'
    let warning: MatchHistoryIssue | undefined
    let allGames: StoredMatchHistoryGame[] = []
    let scannedCount = 0
    try {
      if (!owner.platformId) {
        allGames = await this.nativeGames(owner)
        if (!owner.platformId) throw new HistoryError('platform-unknown')
        source = 'lcu'
      } else {
        try {
          const raw = await this.sgpMatchHistoryService.getHextechAramSummaries(
            owner.puuid, owner.platformId, query.startIndex, query.count, async () => this.canFetch(),
          )
          scannedCount = raw.length
          allGames = raw.flatMap((game) => {
            const normalized = normalizeGame(game, owner.platformId, Date.now())
            return normalized && normalized.platformId === owner.platformId
              && normalized.participants.some((p) => p.puuid === owner.puuid) && isHextechAramGame(normalized) ? [normalized] : []
          })
          if (raw.length && !allGames.length) throw new HistoryError('invalid-response')
        } catch (error) {
          warning = historyIssue(error)
          if (!this.canFetch()) throw new HistoryError('game-busy')
          source = 'lcu'
          allGames = await this.nativeGames(owner)
        }
      }
    } catch (error) {
      if (historyIssue(error) === 'account-changed') throw error
      await this.assertOwner(owner)
      const cached = owner.platformId ? await this.cache.get(owner).catch(() => null) : null
      const issue = historyIssue(error) === 'game-busy' ? 'game-busy' : warning || historyIssue(error)
      if (cached) return this.cachedPage(cached, query, false, issue)
      throw new HistoryError(issue)
    }
    await this.assertOwner(owner)
    const now = Date.now()
    const fullPage = buildHextechAramMatchHistoryPage({ games: allGames, currentPuuid: owner.puuid,
      playerName: owner.playerName, platformId: owner.platformId, ...query, scannedCount, queriedAt: now })
    try { await this.cache.save(owner, fullPage.matches, now) }
    catch { warning = 'cache-write-failed' }
    if (source === 'lcu' && warning !== 'cache-write-failed') {
      const merged = await this.cache.get(owner)
      if (merged) return { ...await this.cachedPage(merged, query, false, warning), source: 'lcu' }
    }
    const page = buildHextechAramMatchHistoryPage({
      games: source === 'lcu' ? allGames.slice(query.startIndex, query.startIndex + query.count) : allGames,
      currentPuuid: owner.puuid, playerName: owner.playerName, platformId: owner.platformId,
      ...query, scannedCount, queriedAt: now, labels: await this.loadLabels(),
    })
    if (source === 'lcu') page.hasMore = query.startIndex + query.count < allGames.length
    return { ...page, source, offline: false, limited: source === 'lcu', warning }
  }

  private canFetch(): boolean {
    const phase = observedHistoryPhase === undefined ? this.lcuService.getConnectionStatus().phase : observedHistoryPhase
    return !!phase && ['None', 'Lobby', 'EndOfGame', 'PreEndOfGame', 'WaitingForStats'].includes(phase)
  }

  private async assertOwner(expected: HistoryOwner): Promise<void> {
    const current = await this.owner()
    if (!current || current.puuid !== expected.puuid || (current.platformId && current.platformId !== expected.platformId)) {
      this.nativeSnapshot = null
      throw new HistoryError('account-changed')
    }
  }

  private async loadLabels(): Promise<HextechAramQueryLabels> {
    try {
      const [champions, augmentsById, items] = await Promise.all([
        loadChampionRoster(),
        loadAugmentDetail(),
        loadItems(),
      ])
      return {
        champions: new Map(champions.map((champion: AnyRecord) => [
          Number(champion.championId ?? champion.id),
          {
            name: asString(champion.nameCN) || asString(champion.nameEN),
            iconUrl: asString(champion.iconUrl),
          },
        ])),
        augments: new Map(Object.entries(augmentsById).map(([id, value]) => {
          const augment = isRecord(value) ? value : {}
          return [Number(id), {
            name: asString(augment.name),
            iconUrl: asString(augment.iconUrl),
          }]
        })),
        items: new Map(items.map((value: unknown) => {
          const item = isRecord(value) ? value : {}
          return [Number(item.id), {
            name: getLocalizedItemName(item),
            iconUrl: asString(item.iconUrl),
          }]
        })),
      }
    } catch (error) {
      logger.debug('[match-history] query labels unavailable:', error instanceof Error ? error.message : String(error))
      return {}
    }
  }
}

let sharedHextechAramQueryService: HextechAramQueryService | null = null

export function getHextechAramQueryService(lcuService: LCUService): HextechAramQueryService {
  if (!sharedHextechAramQueryService) {
    sharedHextechAramQueryService = new HextechAramQueryService(lcuService)
  }
  return sharedHextechAramQueryService
}
