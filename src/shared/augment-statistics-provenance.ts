// A provider may combine Tencent pick rates with independently collected win rates.
// Keep the provenance of the displayed win rate, not the surrounding document.
export function augmentStatisticsProvenance(stats: Record<string, unknown> = {}) {
  const hasWinRateSource = typeof stats.winRateSource === 'string' && stats.winRateSource.length > 0
  return {
    source: 'ARAMGG',
    origin: hasWinRateSource ? stats.winRateSource : stats.source || 'unknown',
    region: (hasWinRateSource ? stats.winRateRegion : stats.region) || 'unknown',
    patch: (hasWinRateSource ? stats.winRateGamePatch : stats.gamePatch) || null,
    date: (hasWinRateSource ? stats.winRateDate : stats.date) || null,
    scope: 'overall' as const,
    stage: null,
  }
}
