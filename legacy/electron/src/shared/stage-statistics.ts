/** Provider stages describe choice order, never a level or an OCR detection count. */
export type AugmentStage = 'all' | '1' | '2' | '3' | '4'
export const AUGMENT_STAGES: AugmentStage[] = ['all', '1', '2', '3', '4']
export interface StageStatRow {
  id: number
  winRate: number
  pickRate: number
  samples: number
}
export interface StageSnapshot {
  schema: 1
  championId: number
  source: 'ARAMKit'
  region: 'unknown'
  patch: string
  dataDate: string
  dataPath: string
  fetchedAt: number
  rows: Record<AugmentStage, StageStatRow[]>
}
export type StageIssue = 'invalid' | 'game-active' | 'busy' | 'unavailable'
export interface StageResult {
  snapshot?: StageSnapshot
  cached?: boolean
  stale?: boolean
  issue?: StageIssue
  referencePatch?: string | null
}

export function validChampionId(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value > 0 && value <= 10000
}

export function visibleStageRows(snapshot: StageSnapshot, stage: AugmentStage): StageStatRow[] {
  // Small samples remain in the cache for inspection, but do not become recommendations.
  return snapshot.rows[stage].filter(row => row.samples >= 200)
}
