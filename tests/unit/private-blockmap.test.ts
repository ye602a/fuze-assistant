import { describe, expect, it } from 'vitest'
import { validateBlockmap } from '../../src/main/update/private-reconstruction.ts'

describe('private update blockmap boundary', () => {
  const valid = () => ({ version: '2' as const, files: [{ name: 'file', offset: 0, sizes: [64, 32], checksums: ['a', 'b'] }] })
  it('accepts a complete single installer map', () => {
    const value = valid()
    expect(validateBlockmap(value, 96)).toBe(value)
  })
  it('rejects inconsistent sizes, counts and nonzero offsets before reconstruction', () => {
    const badSize = valid(); badSize.files[0].sizes[0] = -1
    const badCount = valid(); badCount.files[0].checksums.pop()
    const badOffset = valid(); badOffset.files[0].offset = 1
    for (const value of [badSize, badCount, badOffset]) expect(() => validateBlockmap(value, 96)).toThrow()
    expect(() => validateBlockmap(valid(), 97)).toThrow()
  })
})
