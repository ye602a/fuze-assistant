import { describe, expect, it } from 'vitest'
import { TitleResultCache } from '../../src/main/title-result-cache.ts'
const rows = () => [0, 1, 2].map(() => ({ buffer: Buffer.alloc(120), width: 10, height: 3, channels: 4 }))
describe('bounded exact title reuse', () => {
  it('reuses titles despite a separately captured frame and invalidates a one-byte reroll', () => {
    const cache = new TitleResultCache<string>(); const first = rows()
    cache.set(first, 'cn:patch1', 0, 3000, 'result')
    expect(cache.get(rows(), 'cn:patch1', 500)).toBe('result')
    const changed = rows(); changed[1].buffer[27] = 1
    expect(cache.get(changed, 'cn:patch1', 600)).toBeUndefined()
    expect(cache.get(first, 'cn:patch1', 700)).toBeUndefined()
  })
  it('does not extend TTL on hits or mix locale/catalog/resolution changes', () => {
    const cache = new TitleResultCache<number>(); const input = rows()
    cache.set(input, 'cn:1', 0, 1000, 1)
    expect(cache.get(input, 'cn:1', 999)).toBe(1)
    expect(cache.get(input, 'cn:1', 1000)).toBeUndefined()
    cache.set(input, 'cn:1', 0, 1000, 1)
    expect(cache.get(input, 'en:1', 100)).toBeUndefined()
    cache.set(input, 'cn:1', 0, 1000, 1)
    expect(cache.get(input.map(x => ({ ...x, width: 3, height: 10 })), 'cn:1', 100)).toBeUndefined()
  })
  it('bounds retained pixels and clears selection history', () => {
    const cache = new TitleResultCache<number>()
    const big = [{ buffer: Buffer.alloc(3 * 1024 * 1024), width: 1024, height: 768, channels: 4 }]
    cache.set(big, '', 0, 1000, 1); expect(cache.get(big, '', 1)).toBeUndefined()
    cache.set(rows(), '', 0, 1000, 2); cache.clear(); expect(cache.get(rows(), '', 1)).toBeUndefined()
  })
})
