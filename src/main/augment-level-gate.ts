// Levels grant eligibility, not proof that a player has claimed an augment.
export class AugmentLevelGate {
  level: number | null = null
  checkedAt = 0
  completed = new Set<number>()
  visible = false
  private observedStage: number | null = null
  private absentSince = 0
  private misses = 0
  private lastProbe = 0
  skipped = 0

  reset() {
    this.level = null; this.checkedAt = 0; this.completed.clear()
    this.visible = false; this.observedStage = null; this.absentSince = 0
    this.misses = 0; this.lastProbe = 0; this.skipped = 0
  }

  updateLevel(level: number, now: number) {
    if (!Number.isInteger(level) || level < 1 || level > 30) return
    if (this.level !== null && level < this.level) this.reset()
    this.level = level; this.checkedAt = now
  }

  observeSelection(visible: boolean, now: number) {
    if (visible) {
      this.visible = true; this.absentSince = 0; this.misses = 0
      // Record only the oldest unclaimed entitlement; never infer all earlier picks.
      this.observedStage ??= [3, 7, 11, 15].find(n => n <= (this.level ?? 0) && !this.completed.has(n)) ?? null
    } else if (this.visible) {
      if (!this.misses) this.absentSince = now
      this.misses++
      // Short reroll/title transitions must not put the capture loop to sleep.
      if (this.misses >= 3 && now - this.absentSince >= 6000) {
        if (this.observedStage !== null) this.completed.add(this.observedStage)
        this.observedStage = null; this.visible = false
      }
    }
  }

  decide(now: number) {
    const valid = this.level !== null && now - this.checkedAt <= 10000 && now >= this.checkedAt
    const next = [3, 7, 11, 15].find(n => !this.completed.has(n)) ?? null
    const pending = valid && next !== null && this.level! >= next
    const preparing = valid && next !== null && this.level === next - 1
    const mode = !valid ? 'fallback' : this.visible ? 'selection' : pending ? 'pending' : preparing ? 'preparing' : 'saving'
    // Fail open on unavailable telemetry. Unknown/special offers get an 8-second watchdog.
    const capture = mode !== 'saving' || now - this.lastProbe >= 8000
    if (capture) this.lastProbe = now
    else this.skipped++
    return { mode, capture, prepare: preparing || pending, level: this.level, nextLevel: next,
      checkedAt: this.checkedAt || null, skipped: this.skipped, watchdogMs: 8000 }
  }
}
