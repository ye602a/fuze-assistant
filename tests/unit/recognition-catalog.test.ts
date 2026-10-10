import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import { parseMayhemCatalog, mergeRecognitionCatalog, classifyStatistics, patchKey, selectCoherentStatistics } from '../../src/shared/recognition-catalog.ts'
const payload = JSON.parse(readFileSync(new URL('../../resources/recognition-catalog.json', import.meta.url), 'utf8'))

describe('independent recognition catalog', () => {
  it('validates the actual bundled multilingual public snapshot', () => {
    const catalog = parseMayhemCatalog(payload, 10000)
    expect(catalog.entries.length).toBeGreaterThan(190)
    expect(catalog.patch).toBe('26.20')
    expect(catalog.entries.every(row => row.names['zh-CN'] && row.names['zh-TW'] && row.names['en-US'])).toBe(true)
  })
  it('bridges only unambiguous names; new IDs never become zero or another providers ID', () => {
    const catalog = parseMayhemCatalog(payload, 10000)
    catalog.entries = catalog.entries.slice(0, 2)
    const base = [{ id: 123, name: catalog.entries[0]!.names['zh-CN'], rarity: 1 }]
    const merged = mergeRecognitionCatalog(base, catalog, 'zh-CN')
    expect(merged[0].id).toBe(123)
    expect(merged[1].id).toMatch(/^mayhem:/)
    expect(merged[1].winRate).toBeUndefined()
    expect(merged[1].recommendScore).toBeUndefined()
    const ambiguous = mergeRecognitionCatalog([...base, { ...base[0], id: 456 }], { ...catalog, entries: catalog.entries.slice(0, 1) }, 'zh-CN')
    expect(ambiguous.every(row => !row.recognitionSource)).toBe(true)
  })
  it('rejects partial, duplicate, nonlocalized and malformed downloads', () => {
    for (const broken of [ { ...payload, rows: [] }, { ...payload, license: 'All rights reserved' },
      { ...payload, rows: [...payload.rows, payload.rows[0]] },
      { ...payload, rows: payload.rows.map((row: any) => ({ ...row, name: { en: 'only english' } })) } ]) {
      expect(() => parseMayhemCatalog(broken, 0)).toThrow()
    }
  })
  it('keeps historical, unknown and unavailable statistics distinct from current reference', () => {
    expect(classifyStatistics('16.19', '16.20', true)).toBe('historical')
    expect(classifyStatistics('16.20', '26.20', true)).toBe('matched')
    expect(classifyStatistics(null, '16.20', true)).toBe('unverified')
    expect(classifyStatistics('16.20', '16.20', false)).toBe('missing')
    expect(patchKey('27.20')).toBeNull()
  })
  it('never ranks a newer patch against older or unversioned statistics', () => {
    const rows = ['16.19', '16.20', null].map((patch, id) => ({ id, winRate: .9, recommendScore: .9, statistics: { patch } }))
    expect(selectCoherentStatistics(rows, '16.20').map(row => row.winRate)).toEqual([null, .9, null])
    expect(selectCoherentStatistics(rows, '16.19').map(row => row.winRate)).toEqual([.9, null, null])
  })
})
