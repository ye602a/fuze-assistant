import type { RecognitionCounter, RecognitionFailure, RecognitionSummary } from '../shared/ipc-contract.ts'

export const recognitionCounters: RecognitionCounter[] = ['captures', 'analyses', 'accepted', 'levelWaits', 'unchanged', 'backpressure', 'shards', 'cpuCalls', 'gpuCalls', 'fallbacks', 'recommendations', 'notifications']
const failures: RecognitionFailure[] = ['capture', 'analysis', 'recommendation', 'window', 'ocr']
const reasons = ['gpu-initialization-failed', 'gpu-inference-failed', 'gpu-worker-exited'] as const
/** Whitelist persisted fields; malformed optional summaries never invalidate old journals. */
export function cleanRecognitionSummary(value: unknown): RecognitionSummary | undefined {
  if (!value || typeof value !== 'object') return undefined
  const v = value as RecognitionSummary
  if (!Number.isFinite(v.startedAt) || !Number.isFinite(v.updatedAt) || !v.counts
    || !recognitionCounters.every(key => Number.isSafeInteger(v.counts[key]) && v.counts[key] >= 0)) return undefined
  return { startedAt: v.startedAt, updatedAt: v.updatedAt,
    counts: Object.fromEntries(recognitionCounters.map(key => [key, v.counts[key]])) as RecognitionSummary['counts'],
    lastFailure: v.lastFailure && failures.includes(v.lastFailure.stage) && Number.isFinite(v.lastFailure.at)
      ? { stage: v.lastFailure.stage, at: v.lastFailure.at } : null,
    lastBackend: v.lastBackend === 'cpu' || v.lastBackend === 'webgpu' ? v.lastBackend : null,
    fallbackReason: reasons.includes(v.fallbackReason as typeof reasons[number]) ? v.fallbackReason : null }
}

/** Constant-size counters only: no timers, disk, images, credentials or raw errors. */
export class RecognitionMeter {
  private current: RecognitionSummary | undefined
  private generation = 0
  ticket() { return this.generation }
  begin(previous?: RecognitionSummary) {
    this.generation++
    this.current = cleanRecognitionSummary(previous) || { startedAt: Date.now(), updatedAt: Date.now(),
      counts: Object.fromEntries(recognitionCounters.map(key => [key, 0])) as RecognitionSummary['counts'],
      lastFailure: null, lastBackend: null, fallbackReason: null }
  }
  count(key: RecognitionCounter, ticket = this.generation) {
    if (!this.current || ticket !== this.generation) return
    this.current.counts[key] = Math.min(Number.MAX_SAFE_INTEGER, this.current.counts[key] + 1)
    this.current.updatedAt = Date.now()
  }
  fail(stage: RecognitionFailure, ticket = this.generation) {
    if (!this.current || ticket !== this.generation) return
    this.current.lastFailure = { stage, at: Date.now() }
    this.current.updatedAt = Date.now()
  }
  inference(backend: string, ticket: number) {
    if (!this.current || ticket !== this.generation || !['cpu', 'webgpu'].includes(backend)) return
    this.current.lastBackend = backend as 'cpu' | 'webgpu'
    this.count(backend === 'cpu' ? 'cpuCalls' : 'gpuCalls', ticket)
  }
  fallback(reason: NonNullable<RecognitionSummary['fallbackReason']>, ticket: number) {
    if (!this.current || ticket !== this.generation) return
    this.current.fallbackReason = reason
    this.count('fallbacks', ticket)
  }
  snapshot() { return this.current ? cleanRecognitionSummary(this.current) : undefined }
  finish() { const result = this.snapshot(); this.current = undefined; this.generation++; return result }
}
export const recognitionMeter = new RecognitionMeter()
