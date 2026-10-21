type TitleRow = { buffer: Buffer; width: number; height: number; channels: number }

// One bounded entry, exact bytes; no cryptographic hash, no perceptual collisions.
export class TitleResultCache<T> {
  hits = 0
  private entry: { rows: TitleRow[]; context: string; createdAt: number; until: number; value: T } | null = null
  clear() { this.entry = null }
  get(rows: TitleRow[], context: string, now: number): T | undefined {
    const entry = this.entry
    if (!entry || now < entry.createdAt || now >= entry.until || context !== entry.context) { this.clear(); return }
    if (rows.length !== entry.rows.length || !rows.every((row, i) => {
      const previous = entry.rows[i]
      return row.width === previous.width && row.height === previous.height && row.channels === previous.channels && row.buffer.equals(previous.buffer)
    })) { this.clear(); return }
    this.hits++
    return entry.value
  }
  set(rows: TitleRow[], context: string, now: number, ttl: number, value: T) {
    if (rows.reduce((sum, row) => sum + row.buffer.length, 0) > 2 * 1024 * 1024) { this.clear(); return }
    // Prepared Sharp outputs are immutable and independently owned by the caller.
    this.entry = { rows, context, createdAt: now, until: now + ttl, value }
  }
}
