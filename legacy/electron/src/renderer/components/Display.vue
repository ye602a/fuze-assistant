<template>
  <div class="workbench">
    <aside class="rail">
      <div class="brand"><img class="brand-mark" src="/fuze.svg" alt=""/><div><strong>{{ w('brand') }}</strong><small>FUZE</small></div></div>
      <nav :aria-label="w('workspace')">
        <button v-for="item in navigation" :key="item" :class="{ selected: tab === item }" :aria-label="w(item)" :aria-current="tab === item ? 'page' : undefined" @click="tab = item"><RuneGlyph :name="item"/><span>{{ w(item) }}</span></button>
      </nav>
      <div class="rail-bottom"><span class="local-dot"></span>{{ w('build') }}<small>v{{ status?.version || '—' }}</small></div>
    </aside>
    <div class="content-shell">
      <header class="titlebar"><span>{{ w(tab) }}</span><div class="window-actions"><button :title="w('hide')" :aria-label="w('hide')" @click="electronAPI.windows.toggleMain()"><Minus :size="18"/></button><button :title="w('quit')" :aria-label="w('quit')" @click="quitOpen = true"><X :size="18"/></button></div></header>
      <main>
        <div class="page-heading"><div><p class="eyebrow">{{ w('tagline') }}</p><h1>{{ w(tab === 'overview' ? 'workspace' : tab === 'settings' ? 'preferences' : 'history') }}</h1><p>{{ w(tab === 'settings' ? 'preferencesHint' : 'subtitle') }}</p></div><span class="version-chip">{{ w('brand') }} / {{ w('build') }}</span></div>
        <div v-if="notice" class="notice" role="status">{{ notice }}<button :aria-label="t('common.close')" @click="notice = ''"><X :size="16"/></button></div>
        <template v-if="tab === 'overview'">
          <section class="status-hero" :class="'tone-' + health.tone">
            <div class="status-copy"><div class="eyebrow"><span class="live-dot"/>{{ w('live') }}</div><h2>{{ loadError ? w('unavailable') : status ? w(state) : w('checking') }}</h2><p v-if="status && !loadError">{{ w(state + 'Hint') }}</p><p class="verification">{{ w('verified') }} · {{ formatTime(status?.connection.verifiedAt) }}<span v-if="status?.connection.error"> · {{ status.connection.error }}</span></p></div>
            <button class="primary" :disabled="busy || !status" @click="toggleAutomatic"><Pause v-if="status?.automatic" :size="16"/><Play v-else :size="16"/>{{ w(status?.automatic ? 'pause' : 'resume') }}</button>
          </section>
          <button v-if="loadError" class="secondary" @click="refresh">{{ w('retry') }}</button>
          <p class="footnote" role="note">{{ w('overallBasis') }}</p>
          <section class="pipeline" :aria-label="w('live')">
            <article v-for="item in health.modules" :key="item.key" :class="'tone-' + item.tone">
              <RuneGlyph :name="item.icon"/><small>{{ w(item.key === 'client' ? 'client' : item.key) }}</small>
              <strong><span class="health-dot"/>{{ w(item.label) }}</strong>
              <span v-if="item.key === 'data'" class="data-version">{{ w('dataVersionLabel') }} {{ status?.data.version || '—' }}<br/>{{ w('dataPatch') }} {{ status?.data.patch || '—' }}</span>
              <span>{{ w(item.hint) }}</span>
            </article>
          </section>
          <section class="panel choices-panel"><div class="section-heading"><div><h2>{{ w('recent') }}</h2><p>{{ w('recentHint') }}</p></div><span v-if="status?.recent.detectedAt" class="muted">{{ w('detected', {time: formatTime(status.recent.detectedAt)}) }}</span></div>
            <div class="choices"><article v-for="(card, index) in cards" :key="index" class="choice" :class="{ populated: card && !card.missing }"><div class="choice-index">{{ w('slot', {n: index + 1}) }}</div><RuneGlyph class="rune-mark" name="rune"/><h3>{{ card?.name || w(card ? 'missing' : 'empty') }}</h3><p v-if="card && !card.missing">{{ w('tier') }} <strong>{{ card.tier ? `T${card.tier}` : w('noRating') }}</strong></p><p v-else>{{ w(card?.missing ? 'missingHint' : 'emptyHint') }}</p></article></div>
          </section>
          <section class="panel resource-panel"><div class="section-heading"><h2>{{ w('resources') }}</h2><span class="muted">{{ status?.resources ? formatTime(status.resources.sampledAt) : w('noSample') }}</span></div><div class="metrics"><div><small>{{ w('cpu') }}</small><strong>{{ status?.resources?.cpuPercent.toFixed(1) ?? '—' }}<em>%</em></strong></div><div><small>{{ w('memory') }}</small><strong>{{ status?.resources?.workingSetMB.toFixed(0) ?? '—' }}<em>MB</em></strong></div><div><small>{{ w('captures') }}</small><strong>{{ status?.capture.count ?? '—' }}</strong></div><div><small>{{ w('latency') }}</small><strong>{{ status?.capture.lastAnalysisMs.toFixed(0) ?? '—' }}<em>ms</em></strong></div></div><p class="footnote">{{ w('resourceHint') }}</p></section>
          <details v-if="status?.levelGate" class="panel secondary-detail"><summary>{{ w('levelScheduling') }} · {{ w('levelMode_' + status.levelGate.mode) }}</summary><p class="footnote">{{ w('levelDetail', { level: status.levelGate.level ?? '—', next: status.levelGate.nextLevel ?? '—', skips: status.levelGate.skipped }) }}</p><p class="footnote">{{ w('levelSafety') }}</p></details>
          <details v-if="status?.recognitionData" class="panel secondary-detail"><summary>{{ w('dataChannels') }}</summary><p class="footnote">{{ w('dataStrategy') }}</p><p>{{ w('recognitionCatalog') }} {{ status.recognitionData.catalogPatch || '—' }} · {{ status.recognitionData.count }} · {{ status.recognitionData.catalogDate || '—' }}</p><p>{{ w('statisticsPrimary') }} {{ status.data.patch || '—' }} / {{ status.data.version || '—' }}</p><p>{{ w('referencePatch') }} {{ status.recognitionData.referencePatch || '—' }}</p><p v-for="provider in status.recognitionData.providers" :key="provider.source" class="footnote">{{ provider.source }} · {{ w(provider.error ? 'sourceCached' : provider.checkedAt ? 'sourceChecked' : 'sourcePending') }} · {{ formatTime(provider.checkedAt) }}</p><p class="footnote">{{ status.recognitionData.attribution }}</p></details>
        </template>
        <MatchHistoryPanel v-else-if="tab === 'history'" />
        <template v-else>
          <UpdatePanel />
          <StoragePanel />
          <section class="panel settings-panel"><div class="setting-row"><div><strong>{{ w('automatic') }}</strong><p>{{ w('automaticHint') }}</p></div><button class="switch" role="switch" :aria-label="w('automatic')" :aria-checked="!!status?.automatic" :class="{on: status?.automatic}" :disabled="busy || !status" @click="toggleAutomatic"><span/></button></div><div v-for="pref in preferenceDefinitions" :key="pref.key" class="setting-row"><div><strong>{{ w(pref.label) }}</strong><p>{{ w(pref.label + 'Hint') }}</p></div><button class="switch" role="switch" :aria-label="w(pref.label)" :aria-checked="preferences[pref.key]" :class="{on: preferences[pref.key]}" :disabled="busy" @click="togglePreference(pref.key)"><span/></button></div><div class="setting-row"><div><strong>{{ w('language') }}</strong></div><select :value="locale" :disabled="busy" :aria-label="w('language')" @change="changeLocale"><option value="zh-CN">简体中文</option><option value="zh-TW">繁體中文</option><option value="en-US">English</option></select></div></section>
          <div class="tip"><Monitor :size="20"/><div><strong>{{ w('fullscreen') }}</strong><p>{{ w('fullscreenHint') }}</p></div></div>
          <section class="panel"><h2>{{ w('advanced') }}</h2>
            <div class="setting-row"><div><strong>{{ w('autoClient') }}</strong><p>{{ w('autoClientHint') }}</p></div><span :class="{ good: connected }">{{ w(connected ? 'connected' : 'autoRetry') }}</span></div>
            <p class="footnote">{{ w(status?.clientDiscovery?.processCount ? 'processFound' : 'processWaiting') }} · {{ w('timestamp') }} {{ formatTime(status?.clientDiscovery?.checkedAt) }}</p>
            <p v-for="directory in status?.clientDiscovery?.directories || []" :key="directory" class="detected-directory">{{ w('detectedDirectory') }}：{{ directory }}</p>
            <details class="manual-fallback"><summary>{{ w('manualFallback') }}</summary><label class="path-label" for="league-path">{{ w('path') }}</label><p class="muted">{{ w('pathHint') }}</p><div class="path-control"><input id="league-path" v-model="leaguePath" spellcheck="false"/><button class="secondary" :disabled="busy || !leaguePath.trim()" @click="savePath">{{ w('save') }}</button><button class="secondary" :disabled="busy || !leaguePath.trim()" @click="clearPath">{{ w('clearPath') }}</button></div></details>
            <div class="diagnostic-actions"><button class="secondary" @click="openLogs"><FolderOpen :size="16"/>{{ w('logs') }}</button><button class="secondary" :disabled="busy" @click="preview"><PanelsTopLeft :size="16"/>{{ w('preview') }}</button></div><p class="footnote">{{ w('previewHint') }}</p><StageSignalDiagnostic /></section>
        </template>
        <footer>{{ w('timestamp') }} · {{ status ? formatTime(status.sampledAt) : '—' }} <span>{{ w('brand') }} · {{ w('version') }} {{ status?.version || '—' }}</span></footer>
      </main>
    </div>
    <div v-if="quitOpen" class="modal-backdrop" @click.self="quitOpen = false"><section class="quit-dialog" role="dialog" aria-modal="true" :aria-label="w('quit')"><h2>{{ w('quit') }}</h2><p>{{ w('quitHint') }}</p><div><button class="secondary" @click="quitOpen = false">{{ w('cancel') }}</button><button class="primary" @click="electronAPI.windows.confirmQuit()">{{ w('quit') }}</button></div></section></div>
  </div>
</template>

<script setup lang="ts">
import { computed, defineAsyncComponent, onMounted, onUnmounted, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Minus, X, Play, Pause, Monitor, FolderOpen, PanelsTopLeft } from 'lucide-vue-next'
import RuneGlyph from './RuneGlyph.vue'
import { electronAPI } from '../native/electron-api.ts'
import UpdatePanel from './UpdatePanel.vue'
import StoragePanel from './StoragePanel.vue'
import StageSignalDiagnostic from './StageSignalDiagnostic.vue'
import { setAppLocale } from '../i18n/index.ts'
import { workbenchHealth } from '../../shared/workbench-health.ts'
import { connectionIsVerified, runtimeState, type RuntimeStatus } from '../../shared/runtime-status.ts'
import type { AppStoreKey, SupportedDataLocale } from '../../shared/ipc-contract.ts'
const MatchHistoryPanel = defineAsyncComponent(() => import('./MatchHistoryPanel.vue'))
const { t, locale } = useI18n()
const w = (key: string, params: Record<string, unknown> = {}) => t(`workbench.${key}`, params)
const tab = ref('overview')
const navigation = ['overview', 'history', 'settings']
const status = ref<RuntimeStatus | null>(null)
const loadError = ref(false)
const busy = ref(false)
const notice = ref('')
const quitOpen = ref(false)
const leaguePath = ref('')
const preferenceDefinitions: { key: AppStoreKey; label: string }[] = [{ key: 'ocr.levelGateEnabled', label: 'levelGateOption' }, { key: 'ocr.gpuEnabled', label: 'gpuMode' }, { key: 'augments.showTopOverlay', label: 'top' }, { key: 'augments.showSidePanel', label: 'side' }, { key: 'championInsight.showDetails', label: 'champion' }, { key: 'championInsight.pinDuringChampSelect', label: 'pinChampSelect' }]
const preferences = reactive<Record<string, boolean>>({})
const state = computed(() => status.value ? runtimeState(status.value, status.value.sampledAt) : 'waiting')
const connected = computed(() => !!status.value && connectionIsVerified(status.value.connection, status.value.sampledAt))
const health = computed(() => workbenchHealth(status.value, loadError.value))
const cards = computed(() => [0, 1, 2].map(slot => status.value?.recent.augments.find(card => card.slot === slot)))
function formatTime(value?: number | null) { return value ? new Date(value).toLocaleTimeString(locale.value, { hour12: false }) : w('never') }
let pollTimer: ReturnType<typeof setTimeout> | undefined
let disposed = false
let polling = false
async function refresh() {
  if (polling || disposed) return
  polling = true
  clearTimeout(pollTimer)
  try { status.value = await electronAPI.appInfo.getRuntimeStatus(); loadError.value = false } catch { loadError.value = true }
  finally { polling = false; if (!disposed && !document.hidden) pollTimer = setTimeout(refresh, 2000) }
}
function visibilityChanged() { clearTimeout(pollTimer); if (!document.hidden) void refresh() }
async function action(callback: () => Promise<unknown>) {
  if (busy.value) return
  busy.value = true
  try { await callback() } catch { notice.value = w('failed') } finally { busy.value = false; await refresh() }
}
async function toggleAutomatic() { await action(async () => { const result = await electronAPI.appInfo.setAutomatic(!status.value?.automatic); if (!result.success) throw new Error('failed') }) }
async function togglePreference(key: AppStoreKey) { await action(async () => { const next = !preferences[key]; await electronAPI.store.set(key, next); preferences[key] = next }) }
async function changeLocale(event: Event) { await action(async () => { const next = (event.target as HTMLSelectElement).value as SupportedDataLocale; const result = await electronAPI.locale.set(next); setAppLocale(result.locale) }) }
async function savePath() { await action(async () => { const result = await electronAPI.lcu.setManualLeaguePath(leaguePath.value.trim()); if (!result.success) throw new Error('failed'); notice.value = w('saved') }) }
async function clearPath() { await action(async () => { const result = await electronAPI.lcu.clearManualLeaguePath(); if (!result.success) throw new Error('failed'); leaguePath.value = ''; notice.value = w('saved') }) }
async function openLogs() { await action(async () => { const result = await electronAPI.appInfo.openLogDirectory(); if (!result.success) throw new Error('failed') }) }
async function preview() { await action(async () => { const result = await electronAPI.diagnostics.testShowRandomFloating(); if (!result.success) throw new Error('failed') }) }
const unsubscribe: Array<() => void> = []
onMounted(async () => {
  document.addEventListener('visibilitychange', visibilityChanged)
  unsubscribe.push(electronAPI.events.on('quit-confirm-requested', () => { quitOpen.value = true }))
  void refresh()
  await Promise.allSettled(preferenceDefinitions.map(async pref => { preferences[pref.key] = (await electronAPI.store.get(pref.key)) !== false }))
  try { const result = await electronAPI.lcu.getManualLeaguePath(); leaguePath.value = String(result.path || result.lolPath || '') } catch { /* optional fallback */ }
})
onUnmounted(() => { disposed = true; clearTimeout(pollTimer); document.removeEventListener('visibilitychange', visibilityChanged); unsubscribe.forEach(stop => stop()) })
</script>

<style scoped src="../styles/workbench.css"></style>
