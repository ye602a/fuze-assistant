import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { RecognitionMeter, cleanRecognitionSummary } from '../../src/main/recognition-summary.ts'
import { AugmentJournal } from '../../src/main/services/match-history/augment-journal.ts'
const owner = { puuid: 'fixture-recap-player', platformId: 'HN1', playerName: 'Fixture', gameId: 123 }
const roots: string[] = []
afterEach(async () => { vi.useRealTimers(); for (const root of roots.splice(0)) await rm(root, { recursive: true, force: true }) })
async function fixture() {
  const root = await mkdtemp(path.join(os.tmpdir(), 'fuze-recap-')); roots.push(root)
  const filename = path.join(root, 'journal.json'); const meter = new RecognitionMeter()
  return { filename, meter, journal: new AugmentJournal(filename, meter) }
}
describe('recognition session recap', () => {
  it('has no timers or inactive work and rejects late work from earlier sessions', () => {
    vi.useFakeTimers(); const meter = new RecognitionMeter(); meter.count('captures'); expect(meter.snapshot()).toBeUndefined()
    meter.begin(); const old = meter.ticket(); meter.count('captures'); const first = meter.finish()!
    meter.begin(); meter.count('accepted', old); meter.fail('window', old); meter.fallback('gpu-inference-failed', old)
    expect(first.counts.captures).toBe(1); expect(meter.snapshot()!.counts.accepted).toBe(0); expect(meter.snapshot()!.lastFailure).toBeNull()
    for (let i = 0; i < 10000; i++) meter.count('levelWaits')
    expect(meter.snapshot()!.counts.levelWaits).toBe(10000); expect(vi.getTimerCount()).toBe(0)
  })
  it('counts actual inference attempts and fallback events separately from analysis', () => {
    const meter = new RecognitionMeter(); meter.begin(); const ticket = meter.ticket()
    meter.count('analyses'); meter.inference('webgpu', ticket); meter.fallback('gpu-inference-failed', ticket); meter.inference('cpu', ticket)
    meter.count('analyses') // Reused OCR result: no inference request.
    expect(meter.snapshot()).toMatchObject({ counts: { analyses: 2, gpuCalls: 1, cpuCalls: 1, fallbacks: 1 }, lastBackend: 'cpu' })
  })
  it('whitelists all persisted fields and drops invalid optional recap data', () => {
    const meter = new RecognitionMeter(); meter.begin(); const snapshot = meter.snapshot()!
    expect(cleanRecognitionSummary({ ...snapshot, rawError: 'private-fixture', counts: { ...snapshot.counts, token: 'private-fixture' }, lastFailure: { stage: 'capture', at: 1, error: 'private-fixture' } })).not.toHaveProperty('rawError')
    expect(JSON.stringify(cleanRecognitionSummary({ ...snapshot, rawError: 'private-fixture' }))).not.toContain('private-fixture')
    expect(cleanRecognitionSummary({ ...snapshot, counts: { ...snapshot.counts, captures: -1 } })).toBeUndefined()
  })
  it('persists no-detection games, resumes reconnect counters and isolates the next game', async () => {
    const { filename, meter, journal } = await fixture()
    await journal.onPhase('InProgress', async () => owner); meter.count('levelWaits'); meter.fail('capture')
    await journal.onPhase(null, async () => null)
    await journal.onPhase('InProgress', async () => owner); meter.count('captures')
    await journal.onPhase('EndOfGame', async () => null)
    const saved = (await new AugmentJournal(filename).read(owner))!.sessions[0]
    expect(saved).toMatchObject({ events: [], interrupted: true, recognition: { counts: { levelWaits: 1, captures: 1 }, lastFailure: { stage: 'capture' } } })
    await journal.onPhase('InProgress', async () => ({ ...owner, gameId: 456 }))
    expect((await journal.read(owner))!.sessions[0].recognition!.counts.captures).toBe(0)
    await journal.stop(); expect(meter.snapshot()).toBeUndefined()
  })
  it('keeps old records readable and discards unknown saved recap fields', async () => {
    const { filename, journal } = await fixture()
    await journal.onPhase('InProgress', async () => owner); await journal.onPhase('EndOfGame', async () => null)
    const data = JSON.parse(await readFile(filename, 'utf8')); delete data.sessions[0].recognition
    await writeFile(filename, JSON.stringify(data))
    expect((await new AugmentJournal(filename).read(owner))!.sessions[0].recognition).toBeUndefined()
  })
  it('does not associate metrics with a late or unverifiable session response', async () => {
    const { journal, meter } = await fixture(); let finish!: (value: typeof owner) => void
    const pending = journal.onPhase('InProgress', () => new Promise(resolve => { finish = resolve }))
    await journal.onPhase('EndOfGame', async () => null); finish(owner); await pending
    meter.count('captures'); expect(meter.snapshot()).toBeUndefined(); expect(await journal.read()).toBeNull()
  })
})
