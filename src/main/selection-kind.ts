export type SelectionKind = 'augment' | 'stat-shard' | 'unknown' | 'none'

// Deliberately narrow: unfamiliar or damaged text stays unknown, never completed.
const shardTitle = /^(?:(?:攻击力|攻擊力|法术强度|法術強度|生命值|护甲|護甲|魔法抗性|攻击速度|攻擊速度|技能急速|移动速度|移動速度|暴击几率|暴擊機率|适应之力|適應之力)(?:碎片|属性碎片|屬性碎片)|(?:attack damage|ability power|health|armor|magic resist|attack speed|ability haste|movement speed|critical strike chance|adaptive force) shard)$/i

export function classifySelection(analysis: {
  cardCount?: number; confidence?: number; isAugmentPhase?: boolean
  slotDiagnostics?: Array<{ text?: string; matchedId?: unknown }>
  augmentGate?: { ocrSkippedReason?: unknown; titleActivity?: { likely?: boolean }; rerollButtons?: { visible?: boolean } }
}): SelectionKind {
  if (analysis.augmentGate?.ocrSkippedReason === 'ocr-error') return 'unknown'
  if (analysis.cardCount === 3 && analysis.isAugmentPhase && (analysis.confidence ?? 0) > 0.9) return 'augment'
  const slots = analysis.slotDiagnostics || []
  if (!analysis.cardCount && slots.length === 3 && slots.every(slot =>
    slot.matchedId == null && shardTitle.test((slot.text || '').trim().replace(/\s+/g, ' ')))) return 'stat-shard'
  const gate = analysis.augmentGate
  return (analysis.cardCount || gate?.titleActivity?.likely && gate?.rerollButtons?.visible) ? 'unknown' : 'none'
}
