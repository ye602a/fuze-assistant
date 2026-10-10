import { describe, it, expect } from 'vitest'
import { bitmapToRgba, isRawFrame } from '../../src/shared/raw-frame.ts'
describe('raw capture frames', () => {
  it('swaps Windows BGRA channels without changing alpha or row order', () => {
    const frame = bitmapToRgba(Buffer.from([3, 2, 1, 255, 6, 5, 4, 128]), 1, 2)
    expect([...frame.buffer]).toEqual([1, 2, 3, 255, 4, 5, 6, 128])
    expect(isRawFrame(frame)).toBe(true)
  })
  it('rejects dimensions or strides that could misalign OCR', () => {
    expect(() => bitmapToRgba(Buffer.alloc(5), 1, 1)).toThrow()
    expect(isRawFrame({ buffer: Buffer.alloc(4), width: -1, height: -1, channels: 4 })).toBe(false)
  })
})
