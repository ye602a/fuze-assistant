import type LCUService from '../lcu/lcu-service.ts'
import type { JournalContext } from './augment-journal.ts'
import { historyPlatformFromPayload } from './platform-resolver.ts'
import { validHistoryPuuid } from './personal-history-cache.ts'

/** Called once per InProgress transition, never from the frame/OCR loop. */
export async function readJournalContext(lcu: LCUService): Promise<JournalContext | null> {
  const [session, summoner] = await Promise.all([
    lcu.getReadOnlyJsonEndpoint('/lol-gameflow/v1/session'),
    lcu.getReadOnlyJsonEndpoint('/lol-summoner/v1/current-summoner'),
  ])
  if (session?.status !== 200 || summoner?.status !== 200) return null
  const game = session.data?.gameData
  const player = summoner.data
  if (session.data?.phase !== 'InProgress' || game?.queue?.id !== 2400
    || !Number.isSafeInteger(game.gameId) || game.gameId <= 0 || !validHistoryPuuid(player?.puuid)) return null
  const platformId = historyPlatformFromPayload(player) || await lcu.getCurrentPlatformId()
  if (!platformId) return null
  const gameName = typeof player.gameName === 'string' ? player.gameName : typeof player.displayName === 'string' ? player.displayName : ''
  const tagLine = typeof player.tagLine === 'string' ? player.tagLine : ''
  return { puuid: player.puuid, platformId, playerName: (tagLine ? `${gameName}#${tagLine}` : gameName).slice(0, 128), gameId: game.gameId }
}
