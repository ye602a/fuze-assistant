<template>
  <section class="stage-probe">
    <h3>{{ t('stageSignal.title') }}</h3>
    <p>{{ t('stageSignal.hint') }}</p>
    <label class="scenario">{{ t('stageSignal.scenario') }}<select v-model="scenario" :disabled="busy" @change="clearComparison"><option v-for="item in CLAIM_SCENARIOS" :key="item" :value="item">{{ t('stageSignal.scenario_' + item) }}</option></select></label>
    <button type="button" :disabled="busy" @click="inspect">{{ t(busy ? 'stageSignal.busy' : 'stageSignal.inspect') }}</button>
    <div v-if="report" class="compare-actions"><button type="button" :disabled="busy || report.context?.status !== 'stable'" @click="markBefore">{{ t('stageSignal.markBefore') }}</button><button type="button" @click="clearComparison">{{ t('stageSignal.clear') }}</button></div>
    <p v-if="baseline">{{ t('stageSignal.baseline', { time: new Date(baseline.checkedAt).toLocaleTimeString() }) }}</p>
    <div v-if="comparison" class="comparison" role="status"><strong>{{ t('stageSignal.compare_' + comparison.status) }}</strong><p v-if="comparison.status === 'comparable'">{{ t('stageSignal.changes', { count: comparison.changedFields.length, events: comparison.eventDelta }) }}</p><p>{{ t('stageSignal.noPromotion') }}</p></div>
    <p class="protocol">{{ t('stageSignal.protocol') }}</p>
    <p v-if="notice" role="status">{{ notice }}</p>
    <div v-if="report" class="probe-result" aria-live="polite">
      <strong>{{ t('stageSignal.unknown') }}</strong>
      <p>{{ t('stageSignal.checked', { time: new Date(report.checkedAt).toLocaleTimeString() }) }}</p>
      <ul><li v-for="endpoint in report.endpoints" :key="endpoint.endpoint">{{ t('stageSignal.' + endpoint.endpoint) }} · {{ t('stageSignal.' + endpoint.status) }}<span v-if="endpoint.httpStatus"> ({{ endpoint.httpStatus }})</span></li></ul>
      <p>{{ t('stageSignal.boundary') }}</p>
      <details>
        <summary>{{ t('stageSignal.clues') }}</summary>
        <p>{{ t('stageSignal.level', { level: report.level ?? '—' }) }}</p>
        <p>{{ t('stageSignal.fields', { count: report.schemaFields.length }) }}</p>
        <p v-if="report.schemaFields.length">{{ report.schemaFields.join(', ') }}</p>
        <p v-if="!report.playerFields.length">{{ t('stageSignal.none') }}</p>
        <ul v-else><li v-for="field in report.playerFields" :key="field.field">{{ t('stageSignal.shape', field) }}<span v-if="field.count != null"> · {{ t('stageSignal.count', { count: field.count }) }}</span><span v-if="field.smallInteger != null"> · {{ t('stageSignal.value', { value: field.smallInteger }) }}</span></li></ul>
        <p>{{ t('stageSignal.eventCount', { count: report.augmentEventCount }) }}</p>
      </details>
      <p v-if="report.truncated">{{ t('stageSignal.truncated') }}</p>
    </div>
  </section>
</template>

<script setup lang="ts">
import { onBeforeUnmount, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { electronAPI } from '../native/electron-api.ts'
import { CLAIM_SCENARIOS, compareStageSignals, type StageComparison, type StageSignalReport } from '../../shared/stage-signal.ts'
const { t } = useI18n()
const busy = ref(false)
const notice = ref('')
const report = ref<StageSignalReport | null>(null)
const scenario = ref('normal')
const baseline = ref<StageSignalReport | null>(null)
const comparison = ref<StageComparison | null>(null)
let revision = 0
function clearComparison() { revision++; baseline.value = null; comparison.value = null; report.value = null; notice.value = '' }
function markBefore() { baseline.value = report.value; comparison.value = null }
let mounted = true
onBeforeUnmount(() => { mounted = false })
async function inspect() {
  if (busy.value) return
  busy.value = true; notice.value = ''
  const run = revision
  try {
    const result = await electronAPI.diagnostics.inspectStageSignal()
    if (!mounted || run !== revision) return
    if (result.report) {
      report.value = result.report
      if (baseline.value) {
        comparison.value = compareStageSignals(baseline.value, result.report)
        if (comparison.value.status !== 'comparable') baseline.value = null
      }
    }
    else notice.value = t('stageSignal.cooldown', { seconds: Math.ceil(result.retryAfterMs / 1000) })
  } catch { if (mounted && run === revision) { notice.value = t('stageSignal.failed'); baseline.value = null; report.value = null; comparison.value = null } }
  finally { if (mounted) busy.value = false }
}
</script>

<style scoped>
.stage-probe { margin-top: 20px; padding-top: 16px; border-top: 1px solid #34414f; font-size: 13px; line-height: 1.7; overflow-wrap: anywhere; }
.scenario{display:flex;align-items:center;flex-wrap:wrap;gap:12px;color:#cdbe91;margin:12px 0}.scenario select{padding:7px 10px;background:#09131e;border:1px solid #665638;color:#f0e6d2;max-width:100%}.compare-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:10px}.comparison{padding:12px;border-left:2px solid #c8aa6e;background:#17212a;margin-top:14px}.protocol{font-size:12px}.scenario select:focus-visible,button:focus-visible{outline:2px solid #0ac8b9;outline-offset:3px}
h3 { margin: 0 0 6px; color: #d8e7ec; font-size: 15px; }
p { margin: 8px 0; color: #9aafbc; }
button { padding: 8px 14px; border: 1px solid #577f80; border-radius: 5px; background: #203438; color: #a4e0d6; cursor: pointer; }
button:disabled { opacity: .6; cursor: wait; }
.probe-result { margin-top: 14px; padding: 14px; border-radius: 6px; background: #101c26; color: #cad9df; }
summary { cursor: pointer; color: #a4e0d6; }
ul { padding-left: 20px; }
</style>
