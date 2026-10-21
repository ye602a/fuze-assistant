/** These are search candidates, NOT supported stage adapters. */
export const STAGE_FIELD_NAMES = ['augments', 'selectedAugments', 'augmentSlots', 'augmentIds',
  'augmentStage', 'currentAugmentStage', 'augmentChoiceIndex', 'augmentCount'] as const
export type StageFieldName = typeof STAGE_FIELD_NAMES[number]
export type ProbeStatus = 'ok' | 'http-error' | 'unavailable' | 'timeout' | 'too-large' | 'invalid-json'
export interface StageFieldShape {
  field: StageFieldName
  kind: 'array' | 'object' | 'number' | 'string' | 'boolean' | 'null'
  // No strings, object keys, IDs or array contents cross the diagnostic boundary.
  count?: number
  smallInteger?: number
}
export interface StageSignalReport {
  checkedAt: number
  actualStage: null
  endpoints: Array<{ endpoint: 'schema' | 'player' | 'events'; status: ProbeStatus; httpStatus?: number }>
  schemaFields: StageFieldName[]
  playerFields: StageFieldShape[]
  level: number | null
  augmentEventCount: number
  truncated: boolean
  context?: { revision: number; status: 'stable' | 'changed' | 'unavailable' }
}
export type StageSignalResult = { report: StageSignalReport | null; retryAfterMs: number }

export const CLAIM_SCENARIOS = ['normal', 'reroll', 'shard', 'anvil', 'extra', 'reconnect'] as const
export type ClaimScenario = typeof CLAIM_SCENARIOS[number]
export type StageComparison = { status: 'comparable' | 'expired' | 'context' | 'incomplete'; changedFields: StageFieldName[]; eventDelta: number | null; automaticEligible: false }

// Comparing observations never validates ownership, formal choice kind or a pending choice index.
export function compareStageSignals(before: StageSignalReport, after: StageSignalReport, now = Date.now()): StageComparison {
  const result: StageComparison = { status: 'comparable', changedFields: [], eventDelta: null, automaticEligible: false }
  if (after.checkedAt < before.checkedAt || now < after.checkedAt || now - before.checkedAt > 120000) return { ...result, status: 'expired' }
  if (before.context?.status !== 'stable' || after.context?.status !== 'stable' || before.context.revision !== after.context.revision) return { ...result, status: 'context' }
  if (before.truncated || after.truncated || [before, after].some(r => r.endpoints.length !== 3 || r.endpoints.some(e => e.status !== 'ok'))) return { ...result, status: 'incomplete' }
  for (const field of STAGE_FIELD_NAMES) {
    const a = before.playerFields.find(v => v.field === field), b = after.playerFields.find(v => v.field === field)
    if (a?.kind !== b?.kind || a?.count !== b?.count || a?.smallInteger !== b?.smallInteger) result.changedFields.push(field)
  }
  result.eventDelta = after.augmentEventCount - before.augmentEventCount
  return result
}
