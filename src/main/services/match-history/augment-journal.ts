import { mkdir, readFile, rename, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import type { AugmentJournalEvent, AugmentJournalSession } from '../../../shared/ipc-contract.ts'
import { getMatchHistoryDataDir } from '../../modules/app-paths.ts'
import { HistoryError } from './history-error.ts'
import { validHistoryPuuid, type HistoryOwner } from './personal-history-cache.ts'
import { normalizeHistoryPlatform } from './platform-resolver.ts'

export interface JournalContext extends HistoryOwner { gameId: number }
interface SavedJournal extends AugmentJournalSession { owner: HistoryOwner }
type CardInput = { id?: unknown; missing?: boolean; detectedSlot?: number }
export const JOURNAL_MAX_EVENTS = 80
export const JOURNAL_MAX_SESSIONS = 100
const positiveId = (id: unknown): id is number => typeof id === 'number' && Number.isSafeInteger(id) && id > 0
const sameOwner = (a: HistoryOwner, b: HistoryOwner) => a.puuid === b.puuid && a.platformId === b.platformId

function cleanSession(value: SavedJournal): SavedJournal {
  if (!value || !positiveId(value.gameId) || !validHistoryPuuid(value.owner?.puuid)
    || !normalizeHistoryPlatform(value.owner?.platformId) || typeof value.owner.playerName !== 'string'
    || !Number.isFinite(value.startedAt) || !(value.endedAt === null || Number.isFinite(value.endedAt))
    || !Array.isArray(value.events) || value.events.length > JOURNAL_MAX_EVENTS) throw new Error('invalid journal')
  const events = value.events.map((event): AugmentJournalEvent => {
    if (!event || !Number.isFinite(event.observedAt) || !['first', 'changed', 'reappeared'].includes(event.kind)
      || !Array.isArray(event.cards) || event.cards.length !== 3
      || !event.cards.every(card => card === null || positiveId(card?.id))) throw new Error('invalid event')
    return { observedAt: event.observedAt, kind: event.kind, partial: event.partial === true,
      level: Number.isInteger(event.level) && event.level! >= 1 && event.level! <= 30 ? event.level : null,
      cards: event.cards.map(card => card ? { id: card.id } : null) }
  })
  return { gameId: value.gameId, startedAt: value.startedAt, endedAt: value.endedAt,
    interrupted: value.interrupted === true, capped: value.capped === true, events,
    owner: { puuid: value.owner.puuid, platformId: value.owner.platformId, playerName: value.owner.playerName.slice(0, 128) } }
}

/** Records accepted OCR events only. No capture, network, hashes or selection inference. */
export class AugmentJournal {
  private loaded: Promise<void> | null = null
  private records: SavedJournal[] = []
  private active: SavedJournal | null = null
  private visible = false
  private phase: string | null = null
  private generation = 0
  private dirty = false
  private timer: ReturnType<typeof setTimeout> | null = null
  private writes: Promise<void> = Promise.resolve()
  private writeFailed = false
  private stopped = false
  constructor(private readonly filename = path.join(getMatchHistoryDataDir(), 'augment-journal-v1.json')) {}

  private load(): Promise<void> {
    this.loaded ??= (async () => {
      try {
        if ((await stat(this.filename)).size > 4 * 1024 * 1024) throw new Error('oversized journal')
        const data = JSON.parse(await readFile(this.filename, 'utf8'))
        if (data?.version !== 1 || !Array.isArray(data.sessions) || data.sessions.length > JOURNAL_MAX_SESSIONS) throw new Error('invalid journal')
        this.records = data.sessions.map(cleanSession)
        // An unclosed record from an earlier process is incomplete, never a confirmed result.
        for (const record of this.records) if (record.endedAt === null) record.interrupted = true
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw new HistoryError('cache-unavailable')
      }
    })()
    return this.loaded
  }

  async onPhase(phase: string | null, context: () => Promise<JournalContext | null>): Promise<void> {
    if (this.stopped || this.phase === phase) return
    this.phase = phase
    const generation = ++this.generation
    if (this.active?.events.length) {
      this.active.endedAt = Date.now()
      this.active.interrupted ||= !['WaitingForStats', 'PreEndOfGame', 'EndOfGame'].includes(phase || '')
      this.markDirty()
    }
    this.active = null
    this.visible = false
    if (phase !== 'InProgress') { await this.flush(); return }
    const owner = await context()
    if (!owner || !positiveId(owner.gameId) || !validHistoryPuuid(owner.puuid) || !normalizeHistoryPlatform(owner.platformId)) return
    await this.load()
    if (generation !== this.generation || this.phase !== 'InProgress') return
    const existing = this.records.find(record => record.gameId === owner.gameId && sameOwner(record.owner, owner))
    this.active = existing || { owner: { puuid: owner.puuid, platformId: owner.platformId, playerName: owner.playerName },
      gameId: owner.gameId, startedAt: Date.now(), endedAt: null, interrupted: false, capped: false, events: [] }
    if (existing) { existing.endedAt = null; existing.interrupted = true }
    // Empty sessions do not cause a write or displace older useful records.
  }

  observe(cards: CardInput[], level: number | null, partial = false, observedAt = Date.now()): void {
    if (!this.active || this.phase !== 'InProgress') return
    const slots: AugmentJournalEvent['cards'] = [null, null, null]
    for (const [index, card] of cards.slice(0, 3).entries()) {
      const slot = card.detectedSlot ?? index
      const id = Number(card.id)
      if (Number.isInteger(slot) && slot >= 0 && slot < 3 && !card.missing && positiveId(id)) slots[slot] = { id }
    }
    if (!slots.some(Boolean)) return
    const last = this.active.events.at(-1)
    if (this.visible && last && slots.every((card, slot) => card?.id === last.cards[slot]?.id)) return
    if (this.active.events.length >= JOURNAL_MAX_EVENTS) {
      if (!this.active.capped) { this.active.capped = true; this.markDirty() }
      return
    }
    // Preserve each accepted snapshot, including lost/recovered slots; a change is not proof of a reroll.
    this.active.events.push({ observedAt, level: Number.isInteger(level) && level! >= 1 && level! <= 30 ? level : null,
      kind: !last ? 'first' : this.visible ? 'changed' : 'reappeared', partial: partial || slots.some(card => !card), cards: slots })
    this.visible = true
    this.records = [this.active, ...this.records.filter(record => record !== this.active)].slice(0, JOURNAL_MAX_SESSIONS)
    this.markDirty()
  }

  hide(): void { this.visible = false }
  private markDirty(): void {
    this.dirty = true
    // One bounded-delay write per burst; identical frames never schedule work.
    if (!this.timer) {
      this.timer = setTimeout(() => { this.timer = null; void this.flush().catch(() => {}) }, 2000)
      this.timer.unref?.()
    }
  }

  async flush(): Promise<void> {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    const pending = this.writes.catch(() => {}).then(async () => {
      if (!this.dirty) return
      const body = JSON.stringify({ version: 1, sessions: this.records })
      this.dirty = false
      try {
        await mkdir(path.dirname(this.filename), { recursive: true })
        await writeFile(`${this.filename}.tmp`, body, 'utf8')
        await rename(`${this.filename}.tmp`, this.filename)
        this.writeFailed = false
      } catch { this.dirty = true; this.writeFailed = true; throw new HistoryError('cache-write-failed') }
    })
    this.writes = pending
    return pending
  }

  async read(owner?: HistoryOwner): Promise<{ owner: HistoryOwner; sessions: AugmentJournalSession[] } | null> {
    await this.load()
    if (this.writeFailed) throw new HistoryError('cache-write-failed')
    const target = owner || this.records[0]?.owner
    if (!target) return null
    return { owner: { ...target }, sessions: this.records.filter(record => sameOwner(record.owner, target)).slice(0, 20)
      .map(({ owner: _owner, ...record }) => structuredClone(record)) }
  }

  async stop(): Promise<void> {
    this.stopped = true
    this.generation++
    this.phase = null
    if (this.active?.events.length) { this.active.interrupted = true; this.markDirty() }
    this.active = null
    await this.flush()
  }
}

let shared: AugmentJournal | undefined
export function getAugmentJournal(): AugmentJournal { return shared ??= new AugmentJournal() }
export function recordAugmentObservation(cards: CardInput[], level: number | null, partial: boolean): void {
  shared?.observe(cards, level, partial)
}
export function clearJournalOffer(): void { shared?.hide() }
