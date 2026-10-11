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
}
export type StageSignalResult = { report: StageSignalReport | null; retryAfterMs: number }
