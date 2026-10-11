import { mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { HextechAramMatchHistoryMatch } from '../../../shared/ipc-contract.ts'
import { getMatchHistoryDataDir } from '../../modules/app-paths.ts'
import { HistoryError } from './history-error.ts'
import { normalizeHistoryPlatform } from './platform-resolver.ts'

export interface HistoryOwner { puuid: string; platformId: string; playerName: string }
export interface PersonalHistory extends HistoryOwner {
  updatedAt: number
  matches: HextechAramMatchHistoryMatch[]
}
type CacheData = { version: 1; accounts: PersonalHistory[] }
export const MAX_PERSONAL_MATCHES = 500
const MAX_CACHE_BYTES = 6 * 1024 * 1024

export function validHistoryPuuid(value: unknown): value is string {
  return typeof value === 'string' && /^[a-zA-Z0-9_-]{8,128}$/.test(value)
}

// Persist only the owner's result, IDs and counters; never URLs, other players or auth payloads.
function cleanMatch(value: HextechAramMatchHistoryMatch): HextechAramMatchHistoryMatch | null {
  if (!value || ![value.gameId, value.championId].every((n) => Number.isSafeInteger(n) && n > 0)
    || ![value.gameCreation, value.gameDuration, value.kills, value.deaths, value.assists].every((n) => Number.isFinite(n) && n >= 0)
    || !['win', 'loss', 'remake'].includes(value.result)
    || !Array.isArray(value.augments) || !Array.isArray(value.items)) return null
  const ids = (assets: HextechAramMatchHistoryMatch['items']) => assets
    .filter((asset) => asset && Number.isSafeInteger(asset.id) && asset.id > 0).slice(0, 12).map(({ id }) => ({ id }))
  return {
    gameId: value.gameId, gameCreation: value.gameCreation, gameDuration: value.gameDuration,
    gameVersion: typeof value.gameVersion === 'string' ? value.gameVersion.slice(0, 40) : '',
    championId: value.championId, kills: value.kills, deaths: value.deaths, assists: value.assists,
    result: value.result, augments: ids(value.augments), items: ids(value.items),
  }
}

export class PersonalHistoryCache {
  private loaded: Promise<CacheData> | null = null
  private writes: Promise<unknown> = Promise.resolve()
  constructor(private readonly filename = path.join(getMatchHistoryDataDir(), 'personal-history-v1.json')) {}

  private load(): Promise<CacheData> {
    this.loaded ??= (async () => {
      try {
        if ((await stat(this.filename)).size > MAX_CACHE_BYTES) throw new Error('oversized')
        const data = JSON.parse(await readFile(this.filename, 'utf8')) as CacheData
        if (data.version !== 1 || !Array.isArray(data.accounts) || data.accounts.length > 5) throw new Error('invalid')
        return { version: 1 as const, accounts: data.accounts.filter((account) =>
          validHistoryPuuid(account.puuid) && normalizeHistoryPlatform(account.platformId)
          && typeof account.playerName === 'string' && Number.isFinite(account.updatedAt)
          && Array.isArray(account.matches),
        ).map((account) => ({
          puuid: account.puuid, platformId: account.platformId, playerName: account.playerName.slice(0, 128),
          updatedAt: account.updatedAt,
          matches: account.matches.slice(0, MAX_PERSONAL_MATCHES).flatMap((match) => cleanMatch(match) || []),
        })) }
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { version: 1 as const, accounts: [] }
        // Leave a corrupt file untouched; live queries can still succeed.
        throw new HistoryError('cache-unavailable')
      }
    })()
    return this.loaded
  }

  async get(owner?: Pick<HistoryOwner, 'puuid' | 'platformId'>): Promise<PersonalHistory | null> {
    const data = await this.load()
    const entry = owner ? data.accounts.find((account) => account.puuid === owner.puuid && account.platformId === owner.platformId)
      : [...data.accounts].sort((a, b) => b.updatedAt - a.updatedAt)[0]
    return entry ? structuredClone(entry) : null
  }

  async save(owner: HistoryOwner, matches: HextechAramMatchHistoryMatch[], updatedAt: number): Promise<void> {
    const pending = this.writes.catch(() => {}).then(async () => {
      if (!validHistoryPuuid(owner.puuid) || !normalizeHistoryPlatform(owner.platformId)) throw new HistoryError('platform-unknown')
      const data = await this.load()
      const old = data.accounts.find((entry) => entry.puuid === owner.puuid && entry.platformId === owner.platformId)
      const merged = new Map((old?.matches || []).map((match) => [match.gameId, match]))
      for (const value of matches) {
        const match = cleanMatch(value)
        if (match) merged.set(match.gameId, match)
      }
      const next: CacheData = { version: 1, accounts: [{
        puuid: owner.puuid, platformId: owner.platformId, playerName: owner.playerName.slice(0, 128), updatedAt,
        matches: [...merged.values()].sort((a, b) => b.gameCreation - a.gameCreation).slice(0, MAX_PERSONAL_MATCHES),
      }, ...data.accounts.filter((entry) => entry !== old)].slice(0, 5) }
      try {
        await mkdir(path.dirname(this.filename), { recursive: true })
        await writeFile(`${this.filename}.tmp`, JSON.stringify(next), 'utf8')
        await rename(`${this.filename}.tmp`, this.filename)
        this.loaded = Promise.resolve(next)
      } catch { throw new HistoryError('cache-write-failed') }
    })
    this.writes = pending
    return pending
  }
}
