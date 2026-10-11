<template>
  <section class="stage-probe">
    <h3>{{ t('stageSignal.title') }}</h3>
    <p>{{ t('stageSignal.hint') }}</p>
    <button type="button" :disabled="busy" @click="inspect">{{ t(busy ? 'stageSignal.busy' : 'stageSignal.inspect') }}</button>
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
import type { StageSignalReport } from '../../shared/stage-signal.ts'
const { t } = useI18n()
const busy = ref(false)
const notice = ref('')
const report = ref<StageSignalReport | null>(null)
let mounted = true
onBeforeUnmount(() => { mounted = false })
async function inspect() {
  if (busy.value) return
  busy.value = true; notice.value = ''
  try {
    const result = await electronAPI.diagnostics.inspectStageSignal()
    if (!mounted) return
    if (result.report) report.value = result.report
    else notice.value = t('stageSignal.cooldown', { seconds: Math.ceil(result.retryAfterMs / 1000) })
  } catch { if (mounted) notice.value = t('stageSignal.failed') }
  finally { if (mounted) busy.value = false }
}
</script>

<style scoped>
.stage-probe { margin-top: 20px; padding-top: 16px; border-top: 1px solid #34414f; font-size: 13px; line-height: 1.7; overflow-wrap: anywhere; }
h3 { margin: 0 0 6px; color: #d8e7ec; font-size: 15px; }
p { margin: 8px 0; color: #9aafbc; }
button { padding: 8px 14px; border: 1px solid #577f80; border-radius: 5px; background: #203438; color: #a4e0d6; cursor: pointer; }
button:disabled { opacity: .6; cursor: wait; }
.probe-result { margin-top: 14px; padding: 14px; border-radius: 6px; background: #101c26; color: #cad9df; }
summary { cursor: pointer; color: #a4e0d6; }
ul { padding-left: 20px; }
</style>
