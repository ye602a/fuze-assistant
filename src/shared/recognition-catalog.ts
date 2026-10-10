export interface RecognitionEntry {
  sourceId: string
  names: Record<string, string>
  rarity: number
  pickRate: number | null
}
export interface RecognitionCatalog {
  schema: 1
  source: 'arammayhem.com'
  patch: string
  statsDate: string
  retrievedAt: number
  attribution: string
  entries: RecognitionEntry[]
}

export function patchKey(value: unknown): string | null {
  const match = /^(16|26)\.(\d{1,2})(?:\.|$)/.exec(String(value || ''))
  return match ? `2026.${Number(match[2])}` : null
}

export function parseMayhemCatalog(value: unknown, now: number): RecognitionCatalog {
  const data = value as Record<string, any>
  if (!String(data?.license).startsWith('CC BY 4.0') || data?.licenseUrl !== 'https://creativecommons.org/licenses/by/4.0/' || !patchKey(data?.meta?.patch) ||
      !/^\d{4}-\d{2}-\d{2}$/.test(data?.meta?.statsDate || '') || !Array.isArray(data?.rows) ||
      data.rows.length < 50 || data.rows.length > 2000) throw new Error('Invalid recognition catalog metadata')
  const ids = new Set<string>()
  const entries = data.rows.map((row: any) => {
    if (typeof row.augmentId !== 'string' || !/^[a-zA-Z0-9_!-]{1,100}$/.test(row.augmentId) || ids.has(row.augmentId)) throw new Error('Invalid catalog identifier')
    ids.add(row.augmentId)
    const names: Record<string, string> = {}
    for (const locale of ['zh-CN', 'zh-TW', 'en']) {
      const name = row.name?.[locale]
      if (typeof name !== 'string' || !name.trim() || name.length > 160 || /[<>]/.test(name) || [...name].some(c => c.charCodeAt(0) < 32)) throw new Error('Invalid localized augment name')
      names[locale === 'en' ? 'en-US' : locale] = name.trim()
    }
    const rarity = ({ silver: 0, gold: 1, prismatic: 2 } as Record<string, number>)[row.rarity]
    if (rarity === undefined) throw new Error('Invalid augment rarity')
    const pickRate = typeof row.pickRate === 'number' && row.pickRate >= 0 && row.pickRate <= 100 ? row.pickRate / 100 : null
    return { sourceId: row.augmentId, names, rarity, pickRate }
  })
  return { schema: 1, source: 'arammayhem.com', patch: data.meta.patch, statsDate: data.meta.statsDate,
    retrievedAt: now, attribution: 'Data: arammayhem.com · CC BY 4.0 · https://arammayhem.com/zh-cn/data/', entries }
}

const nameKey = (name: string) => name.normalize('NFKC').replace(/[\s：:·.,，。！!？?-]/g, '').toLowerCase()

// Cross-provider IDs are NOT interchangeable. Bridge unique exact names only.
export function mergeRecognitionCatalog(base: any[], catalog: RecognitionCatalog | null, locale: string) {
  if (!catalog) return base
  const byName = new Map<string, any[]>()
  for (const item of base) {
    const key = nameKey(String(item.name || ''))
    byName.set(key, [...(byName.get(key) || []), item])
  }
  const result = new Map(base.map(row => [String(row.id), row]))
  for (const entry of catalog.entries) {
    const name = entry.names[locale]
    if (!name) continue
    const matches = byName.get(nameKey(name)) || []
    if (matches.length > 1) continue
    const previous = matches[0]
    const id = previous?.id ?? `mayhem:${entry.sourceId}`
    result.set(String(id), { ...previous, id, name, rarity: ['kSilver', 'kGold', 'kPrismatic'][entry.rarity],
      recognitionSource: catalog.source, recognitionPatch: catalog.patch,
      popularity: entry.pickRate, popularityPatch: catalog.patch,
      // No score, win rate or global recommendation is derived from popularity.
    })
  }
  return [...result.values()]
}

export function classifyStatistics(patch: unknown, reference: unknown, hasStats: boolean) {
  if (!hasStats) return 'missing'
  const a = patchKey(patch); const b = patchKey(reference)
  return !a || !b ? 'unverified' : a === b ? 'matched' : 'historical'
}

export function selectCoherentStatistics(rows: any[], reference: unknown): any[] {
  const patches = [...new Set(rows.map(row => patchKey(row.statistics?.patch)).filter(Boolean))] as string[]
  if (!patches.length) return rows
  const target = patchKey(reference)
  const selected = target && patches.includes(target) ? target : patches.sort((a, b) => Number(b.split('.')[1]) - Number(a.split('.')[1]))[0]
  return rows.map(row => patchKey(row.statistics?.patch) === selected ? row : {
    ...row, tier: null, rank: null, winRate: null, win_rate: null, pickRate: null, pick_rate: null,
    playCount: null, num_games: null, winCount: null, num_win_games: null, recommendScore: null,
    statistics: { ...row.statistics, status: 'missing' },
  })
}
