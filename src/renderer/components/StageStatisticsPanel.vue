<template>
  <details class="stage-panel" @toggle="toggle">
    <summary><span class="stage-emblem" aria-hidden="true">IV</span><span><strong>{{ t('stageStats.title') }}</strong><small>{{ t('stageStats.subtitle') }}</small></span><span class="expand" aria-hidden="true">+</span></summary>
    <div v-if="open" class="stage-body">
      <p class="hint">{{ t('stageStats.hint') }}</p>
      <p class="hint">{{ t('stageStats.boundary') }}</p>
      <p v-if="loading" role="status">{{ t('stageStats.loading') }}</p>
      <p v-if="result?.issue" role="alert" class="notice">{{ t(`stageStats.issues.${result.issue}`) }}</p>
      <template v-if="snapshot">
        <div class="provenance"><strong>{{ t('stageStats.source') }}</strong><span>{{ t('stageStats.metadata', { patch: snapshot.patch, date: snapshot.dataDate }) }}</span><span v-if="result?.cached">{{ t('stageStats.cached') }}</span></div>
        <p v-if="result?.stale" class="notice">{{ t('stageStats.stale') }}</p>
        <p v-if="result?.referencePatch && result.referencePatch !== snapshot.patch" class="notice">{{ t('stageStats.mismatch') }}</p>
        <p class="hint">{{ t('stageStats.unknownPatch') }}</p>
        <div class="stages" role="group" :aria-label="t('stageStats.title')"><button v-for="key in AUGMENT_STAGES" :key="key" type="button" :aria-pressed="stage === key" @click="selectStage(key)">{{ key === 'all' ? t('stageStats.all') : t('stageStats.stage', { stage: key }) }}</button></div>
        <div v-if="rows.length" class="table-scroll"><table><thead><tr><th>{{ t('stageStats.augment') }}</th><th>{{ t('stageStats.win') }}</th><th>{{ t('stageStats.pick') }}</th><th>{{ t('stageStats.sample') }}</th></tr></thead><tbody><tr v-for="row in rows.slice(0, limit)" :key="row.id"><td>{{ names.get(row.id) || t('stageStats.fallbackName', { id: row.id }) }}</td><td class="win">{{ percent(row.winRate) }}</td><td>{{ percent(row.pickRate) }}</td><td>{{ number.format(row.samples) }}</td></tr></tbody></table></div>
        <p v-else class="empty">{{ t('stageStats.empty') }}</p>
        <button v-if="rows.length > limit" class="more" type="button" @click="limit += 25">{{ t('stageStats.more') }}</button>
        <p class="hint note">{{ t('stageStats.note') }}</p>
      </template>
      <button v-if="!loading && (!snapshot || result?.stale || result?.issue)" type="button" class="retry" @click="load(true)">{{ t(snapshot ? 'stageStats.refresh' : 'stageStats.retry') }}</button>
    </div>
  </details>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { AUGMENT_STAGES, visibleStageRows, type AugmentStage, type StageResult } from '../../shared/stage-statistics.ts'
import { electronAPI } from '../native/electron-api.ts'
const props = defineProps<{ championId: number | string; augments: Array<{ id?: number | string; augmentId?: number | string; name?: string }> }>()
const { t, locale } = useI18n()
const open = ref(false), loading = ref(false), stage = ref<AugmentStage>('all'), limit = ref(25)
const result = ref<StageResult>()
let revision = 0
const snapshot = computed(() => result.value?.snapshot)
const rows = computed(() => snapshot.value ? visibleStageRows(snapshot.value, stage.value) : [])
const names = computed(() => new Map(props.augments.map(row => [Number(row.augmentId ?? row.id), row.name])))
const number = computed(() => new Intl.NumberFormat(locale.value))
const rate = computed(() => new Intl.NumberFormat(locale.value, { style: 'percent', minimumFractionDigits: 1, maximumFractionDigits: 1 }))
const percent = (value: number) => rate.value.format(value)
function selectStage(value: AugmentStage) { stage.value = value; limit.value = 25 }
async function load(refresh = false) {
  if (loading.value || !open.value) return
  const run = ++revision, id = Number(props.championId)
  loading.value = true
  try {
    const response = await electronAPI.stageStatistics.get(id, refresh)
    if (run === revision && open.value && id === Number(props.championId)) result.value = response
  } catch { if (run === revision) result.value = { ...result.value, issue: 'unavailable' } }
  finally { if (run === revision) loading.value = false }
}
function toggle(event: Event) {
  if (event.target !== event.currentTarget) return
  open.value = (event.currentTarget as HTMLDetailsElement).open
  if (open.value) void load()
  else { revision++; loading.value = false; result.value = undefined; selectStage('all') }
}
watch(() => props.championId, () => { revision++; loading.value = false; result.value = undefined; selectStage('all') })
onUnmounted(() => { revision++ })
</script>

<style scoped>
.stage-panel{margin:18px 0;border:1px solid #665838;border-radius:10px;background:linear-gradient(125deg,#1d2d37,#131e29);color:#e5e8e9;font-size:13px}.stage-panel summary{display:flex;align-items:center;gap:14px;padding:17px;list-style:none;cursor:pointer}.stage-panel summary::-webkit-details-marker{display:none}.stage-emblem{display:grid;place-items:center;width:38px;height:38px;border:1px solid #9e8955;transform:rotate(-5deg);font:18px Georgia,serif;color:#e3c589}.stage-panel strong{font-weight:600}.stage-panel small{display:block;color:#a9b8c5;font-size:11px;margin-top:5px}.expand{margin-left:auto;font-size:22px;color:#d8c38f}.stage-panel[open] .expand{transform:rotate(45deg)}.stage-body{padding:0 17px 18px}.hint{color:#a9b8c5;font-size:12px;line-height:1.7;margin:8px 0}.notice{color:#e4c18a;font-size:12px;line-height:1.7}.provenance{display:flex;flex-wrap:wrap;gap:7px 16px;border-top:1px solid #344551;padding-top:14px;margin-top:15px;font-size:11px;color:#a9b8c5}.provenance strong{color:#d9c592}.stages{display:flex;flex-wrap:wrap;gap:6px;margin:15px 0}button{border:1px solid #43576a;background:#1a2a38;color:#c3d0dd;border-radius:6px;padding:8px 13px;font:inherit;cursor:pointer}button[aria-pressed=true]{background:#c9b27b;color:#17212c;border-color:#dfcb96;font-weight:600}button:focus-visible,summary:focus-visible{outline:2px solid #76d6c5;outline-offset:3px}.table-scroll{overflow:auto;max-height:340px;border:1px solid #344551;border-radius:7px}table{width:100%;border-collapse:collapse;font-variant-numeric:tabular-nums;white-space:nowrap}th{position:sticky;top:0;background:#20313c;color:#aebfc9;font-size:11px;font-weight:500}td,th{padding:11px 10px;text-align:right;border-bottom:1px solid #2a3b48}td:first-child,th:first-child{text-align:left}td:first-child{max-width:220px;overflow:hidden;text-overflow:ellipsis}.win{font-size:17px;color:#98d9c9}.more,.retry{margin-top:12px}.note{margin-top:15px}.empty{padding:20px 0;color:#b4c4d3;line-height:1.7}
</style>
