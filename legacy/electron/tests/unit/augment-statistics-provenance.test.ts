import { describe, expect, it } from 'vitest'
import { augmentStatisticsProvenance } from '../../src/shared/augment-statistics-provenance.ts'
describe('win rate provenance and stage fallback', () => {
  it('does not label client-uploaded win rates as Tencent CN or assign a stage', () => {
    expect(augmentStatisticsProvenance({ source: 'tencent', region: 'CN', gamePatch: '16.19',
      winRateSource: 'aramgg-client-upload', winRateRegion: 'WORLD', winRateGamePatch: '16.20' }))
      .toMatchObject({ origin: 'aramgg-client-upload', region: 'WORLD', patch: '16.20', scope: 'overall', stage: null })
  })
  it('leaves missing specific metadata unknown instead of borrowing a different population', () => {
    expect(augmentStatisticsProvenance({ source: 'tencent', region: 'CN', gamePatch: '16.19', winRateSource: 'client' }))
      .toMatchObject({ region: 'unknown', patch: null, date: null })
    expect(augmentStatisticsProvenance({ source: 'tencent', region: 'CN', gamePatch: '16.19' }))
      .toMatchObject({ region: 'CN', patch: '16.19' })
  })
})
