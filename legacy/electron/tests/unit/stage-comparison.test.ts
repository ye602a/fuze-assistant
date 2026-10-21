import { describe, expect, it } from 'vitest'
import { compareStageSignals, type StageSignalReport } from '../../src/shared/stage-signal.ts'
import { StageSignalProbe } from '../../src/main/services/stage-signal.ts'
const report = (overrides: Partial<StageSignalReport> = {}): StageSignalReport => ({
  checkedAt: 1000, actualStage: null, level: 11, truncated: false, schemaFields: [],
  endpoints: ['schema', 'player', 'events'].map(endpoint => ({ endpoint: endpoint as 'schema' | 'player' | 'events', status: 'ok', httpStatus: 200 })),
  playerFields: [{ field: 'augmentIds', kind: 'array', count: 1 }], augmentEventCount: 1,
  context: { status: 'stable', revision: 4 }, ...overrides,
})
describe('manual claim evidence comparisons', () => {
  it('reports increases and replacements only as clues, not formal stages', () => {
    const before = report()
    const after = report({ checkedAt: 31000, level: 15, playerFields: [{ field: 'augmentIds', kind: 'array', count: 4 }], augmentEventCount: 5 })
    expect(compareStageSignals(before, after, 31000)).toEqual({ status: 'comparable', changedFields: ['augmentIds'], eventDelta: 4, automaticEligible: false })
    // A reroll/replacement may leave counts identical. That cannot establish an unchanged choice.
    expect(compareStageSignals(after, { ...after, checkedAt: 61000 }, 61000).changedFields).toEqual([])
    expect(compareStageSignals(after, { ...after, checkedAt: 61000, augmentEventCount: 0 }, 61000).eventDelta).toBe(-5)
  })
  it('rejects interrupted sessions, unavailable contexts, incomplete responses and stale samples', () => {
    expect(compareStageSignals(report(), report({ context: { status: 'stable', revision: 5 } }), 1000).status).toBe('context')
    expect(compareStageSignals(report(), report({ context: undefined }), 1000).status).toBe('context')
    expect(compareStageSignals(report(), report({ truncated: true }), 1000).status).toBe('incomplete')
    expect(compareStageSignals(report(), report({ endpoints: [] }), 1000).status).toBe('incomplete')
    expect(compareStageSignals(report(), report({ checkedAt: 122000 }), 122000).status).toBe('expired')
    expect(compareStageSignals(report(), report({ checkedAt: 500 }), 1000).status).toBe('expired')
  })
  it('discards player evidence from a probe spanning a phase/auth transition', async () => {
    let revision = 1
    const probe = new StageSignalProbe(async () => {
      revision++
      return { status: 'ok', httpStatus: 200, data: { level: 15, augmentStage: 3 } }
    }, () => 1000, () => ({ revision, usable: true }))
    const result = (await probe.inspect()).report!
    expect(result.context?.status).toBe('changed')
    expect(result.playerFields).toEqual([])
    expect(result.level).toBeNull()
    expect(result.actualStage).toBeNull()
  })
})
