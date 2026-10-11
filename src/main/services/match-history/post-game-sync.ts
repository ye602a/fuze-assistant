/** A single delayed refresh after an observed game; no startup scan or repeating timer. */
export class PostGameHistorySync {
  private pending = false
  private timer: ReturnType<typeof setTimeout> | null = null
  constructor(private readonly refresh: () => Promise<unknown>) {}

  onPhase(phase: string | null): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    if (phase === 'InProgress') { this.pending = true; return }
    if (phase === 'ChampSelect' || phase === 'GameStart' || !phase) { this.pending = false; return }
    if (!this.pending || !['Lobby', 'None', 'EndOfGame'].includes(phase)) return
    this.timer = setTimeout(() => {
      this.timer = null
      this.pending = false
      void this.refresh().catch(() => { /* On-demand refresh remains available. */ })
    }, 10_000)
    this.timer.unref?.()
  }

  stop(): void {
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
    this.pending = false
  }
}
