<template>
  <section class="recognition-recap">
    <h4>{{ t('matchHistory.journal.recap.title') }}</h4>
    <p v-if="!summary">{{ t('matchHistory.journal.recap.missing') }}</p>
    <template v-else>
      <div class="recap-counts">
        <div v-for="key in keys" :key="key"><strong>{{ summary.counts[key] }}</strong><span>{{ t(`matchHistory.journal.recap.counts.${key}`) }}</span></div>
      </div>
      <p v-if="!summary.counts.accepted" class="recap-notice">{{ t('matchHistory.journal.recap.noAccepted') }}</p>
      <p>{{ t('matchHistory.journal.recap.lastFailure') }} · {{ summary.lastFailure ? t(`matchHistory.journal.recap.stages.${summary.lastFailure.stage}`) : t('matchHistory.journal.recap.noFailure') }}<time v-if="summary.lastFailure"> · {{ formatTime(summary.lastFailure.at) }}</time></p>
      <p v-if="summary.fallbackReason">{{ t('matchHistory.journal.recap.fallback') }} · {{ t(`matchHistory.journal.recap.reasons.${summary.fallbackReason}`) }}</p>
      <p>{{ t('matchHistory.journal.recap.scope') }} <span v-if="summary.counts.notifications">{{ t('matchHistory.journal.recap.sent') }}</span></p>
    </template>
  </section>
</template>
<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import type { RecognitionCounter, RecognitionSummary } from '../../shared/ipc-contract.ts'
defineProps<{ summary?: RecognitionSummary }>()
const { t, locale } = useI18n()
const keys: RecognitionCounter[] = ['captures', 'analyses', 'accepted', 'notifications', 'levelWaits', 'unchanged', 'backpressure', 'shards', 'cpuCalls', 'gpuCalls', 'fallbacks', 'recommendations']
const formatTime = (value: number) => new Intl.DateTimeFormat(locale.value, { hour: '2-digit', minute: '2-digit', second: '2-digit' }).format(value)
</script>
<style scoped>
.recognition-recap { margin: 10px; padding: 12px; border: 1px solid #40505a; border-radius: 5px; background: #101f28; }
h4 { color: #dfc58a; margin: 0 0 12px; font-size: 14px; }
.recap-counts { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
.recap-counts div { padding: 10px 6px; background: #172a33; border-radius: 4px; text-align: center; min-width: 0; }
strong { display: block; color: #d6e6e9; font-size: 22px; font-variant-numeric: tabular-nums; }
span, p { font-size: 12px; line-height: 1.6; overflow-wrap: anywhere; }
p { color: #a7b8c1; margin: 10px 0 0; }
.recap-notice { color: #e4c994; }
@media(max-width: 620px) { .recap-counts { grid-template-columns: repeat(3, minmax(0, 1fr)); } }
</style>
