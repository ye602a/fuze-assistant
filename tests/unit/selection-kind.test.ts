import { describe, expect, it } from 'vitest'
import { classifySelection } from '../../src/main/selection-kind.ts'

const slots = (texts: string[]) => texts.map(text => ({ text, matchedId: null }))
describe('selection identity without guessing unknown offers', () => {
  it('keeps actual Stats augments as formal choices', () => {
    expect(classifySelection({ cardCount: 3, confidence: 0.99, isAugmentPhase: true,
      slotDiagnostics: slots(['属性！', '属性叠属性！', '属性叠属性叠属性！']) })).toBe('augment')
  })
  it('recognizes only three explicit shard titles, including English and traditional Chinese', () => {
    expect(classifySelection({ cardCount: 0, slotDiagnostics: slots(['攻击力碎片', '法術強度碎片', 'Health Shard']) })).toBe('stat-shard')
    expect(classifySelection({ cardCount: 0, slotDiagnostics: slots(['属性！', '未知', '攻击力碎片']) })).not.toBe('stat-shard')
  })
  it('does not complete from UI geometry or partial OCR', () => {
    expect(classifySelection({ cardCount: 0, augmentGate: { titleActivity: { likely: true }, rerollButtons: { visible: true } } })).toBe('unknown')
    expect(classifySelection({ cardCount: 2, confidence: 0.99, isAugmentPhase: true })).toBe('unknown')
    expect(classifySelection({ cardCount: 3, confidence: 0.7, isAugmentPhase: true })).toBe('unknown')
    expect(classifySelection({ cardCount: 0 })).toBe('none')
    expect(classifySelection({ cardCount: 0, augmentGate: { ocrSkippedReason: 'ocr-error' } })).toBe('unknown')
  })
})
