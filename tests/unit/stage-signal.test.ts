import { describe, expect, it, vi } from 'vitest'
import https from 'node:https'
import { EventEmitter } from 'node:events'
import { StageSignalProbe, readStageProbe, summarizeStageProbe, type ProbeResponse, PROBE_PATHS } from '../../src/main/services/stage-signal.ts'
const ok = (data: unknown): ProbeResponse => ({ status: 'ok', httpStatus: 200, data })
describe('stage signal inspection', () => {
  it('terminates a stalled local request without leaking error details', async () => {
    vi.useFakeTimers()
    const request = Object.assign(new EventEmitter(), { destroy: vi.fn() })
    const spy = vi.spyOn(https, 'get').mockReturnValue(request as unknown as ReturnType<typeof https.get>)
    try {
      const pending = readStageProbe(PROBE_PATHS[1])
      await vi.advanceTimersByTimeAsync(1500)
      expect(await pending).toEqual({ status: 'timeout' })
      expect(request.destroy).toHaveBeenCalledTimes(1)
      request.emit('error', new Error('PRIVATE_ERROR'))
      expect(await pending).toEqual({ status: 'timeout' })
      expect(vi.getTimerCount()).toBe(0)
    } finally { spy.mockRestore(); vi.useRealTimers() }
  })
  it('exports only allowlisted shapes, never identities, raw IDs or string values', () => {
    const secret = 'PRIVATE_FIXTURE_VALUE'
    const report = summarizeStageProbe([
      ok({ components: { schemas: { Player: { properties: { augmentStage: { type: 'integer' }, [secret]: {} } } } }, examples: { properties: { augmentSlots: {} } } }),
      ok({ level: 11, summonerName: secret, puuid: secret, augmentStage: 2, augmentIds: [1001, 1002], selectedAugments: [{ id: 1001, name: secret }], augments: secret, nested: { augmentCount: 4 } }),
      ok({ Events: [{ EventName: 'AugmentSelected', SummonerName: secret }, { EventName: secret }] }),
    ], 123)
    expect(report.actualStage).toBeNull()
    expect(report.schemaFields).toEqual(['augmentStage'])
    expect(report.playerFields).toContainEqual({ field: 'augmentStage', kind: 'number', smallInteger: 2 })
    expect(report.playerFields).toContainEqual({ field: 'augmentIds', kind: 'array', count: 2 })
    expect(report.playerFields.some(x => x.field === 'augmentCount')).toBe(false)
    expect(report.augmentEventCount).toBe(1)
    expect(JSON.stringify(report)).not.toContain(secret)
    expect(JSON.stringify(report)).not.toContain('1001')
  })
  it.each([3, 7, 11, 15, 18])('never infers a stage from level %i, count, events or a plausible field', level => {
    const report = summarizeStageProbe([ok({}), ok({ level, augmentCount: 4, augmentStage: 3 }), ok({ Events: [{ EventName: 'AugmentGranted' }] })], 1)
    expect(report.actualStage).toBeNull()
  })
  it('ignores error response bodies and caps arrays, events and schema traversal', () => {
    const schema: Record<string, unknown> = {}; schema.loop = schema
    const report = summarizeStageProbe([ok(schema), ok({ augmentIds: Array(100).fill('hidden'), augmentStage: 887 }), ok({ Events: Array(3000).fill({ EventName: 'AugmentGranted' }) })], 1)
    expect(report.truncated).toBe(true)
    expect(report.playerFields.find(x => x.field === 'augmentIds')?.count).toBe(32)
    expect(report.playerFields.find(x => x.field === 'augmentStage')?.smallInteger).toBeUndefined()
    expect(report.augmentEventCount).toBe(2048)
    expect(summarizeStageProbe([{ status: 'http-error', httpStatus: 401, data: { properties: { augments: {} } } }], 1).schemaFields).toEqual([])
  })
  it('does no startup I/O, deduplicates, cools down failures and does not reuse previous reports', async () => {
    let now = 0
    let finish!: (value: ProbeResponse) => void
    const read = vi.fn().mockImplementationOnce(() => new Promise(resolve => { finish = resolve })).mockResolvedValue(ok({}))
    const probe = new StageSignalProbe(read, () => now)
    expect(read).not.toHaveBeenCalled()
    const first = probe.inspect(); expect(probe.inspect()).toBe(first)
    finish(ok({ properties: { augmentStage: {} } }))
    expect((await first).report?.schemaFields).toEqual(['augmentStage'])
    expect(read.mock.calls.map(x => x[0])).toEqual([...PROBE_PATHS])
    expect(await probe.inspect()).toEqual({ report: null, retryAfterMs: 30000 })
    now = 30000; read.mockRejectedValue(new Error('PRIVATE_ERROR'))
    const next = await probe.inspect()
    expect(next.report?.schemaFields).toEqual([])
    expect(next.report?.endpoints.every(x => x.status === 'unavailable')).toBe(true)
    expect(JSON.stringify(next)).not.toContain('PRIVATE_ERROR')
    expect(await probe.inspect()).toEqual({ report: null, retryAfterMs: 30000 })
  })
})
