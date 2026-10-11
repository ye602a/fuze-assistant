<template>
  <details class="storage-panel" @toggle="toggle">
    <summary><HardDrive :size="22" /><span><strong>{{ t('storage.title') }}</strong><small>{{ t('storage.subtitle') }}</small></span></summary>
    <div v-if="open" class="storage-body">
      <p class="muted">{{ t('storage.hint') }}</p>
      <div class="protected"><ShieldCheck :size="19" /><p>{{ t('storage.protected') }}</p></div>
      <div class="toolbar"><button type="button" :disabled="!!busy" @click="scan"><RefreshCw :size="15" />{{ t(busy === 'clean' ? 'storage.cleaning' : busy === 'scan' ? 'storage.scanning' : 'storage.scan') }}</button></div>
      <p v-if="error" role="alert" class="warning">{{ t(`storage.errors.${error}`) }}</p>
      <p v-if="message" role="status" class="notice">{{ message }}</p>
      <template v-if="preview">
        <div class="totals"><div><small>{{ t('storage.total') }}</small><strong>{{ size(total) }}</strong></div><div><small>{{ t('storage.reclaimable') }}</small><strong>{{ size(preview.reclaimableBytes) }}</strong></div></div>
        <ul class="categories"><li v-for="category in preview.categories" :key="category.kind"><span>{{ t(`storage.${category.kind}`) }}<small>{{ t('storage.files', { count: category.files }) }}</small></span><strong>{{ size(category.bytes) }}</strong></li></ul>
        <p v-if="preview.skipped" class="warning">{{ t('storage.skipped', { count: preview.skipped }) }}</p>
        <p v-if="preview.protectionUnknown" class="warning">{{ t('storage.unknown') }}</p>
        <div class="cleanup-preview"><h3>{{ t('storage.candidates', { count: preview.candidates.length }) }}</h3><p class="muted">{{ t('storage.policy') }}</p>
          <ul v-if="preview.candidates.length" class="candidates"><li v-for="(candidate, index) in preview.candidates" :key="index"><div><span>{{ candidate.label }}</span><small>{{ t(`storage.${candidate.kind}`) }} · {{ t('storage.files', { count: candidate.files }) }}</small></div><strong>{{ size(candidate.bytes) }}</strong></li></ul>
          <p v-else class="empty">{{ t('storage.empty') }}</p>
          <button v-if="preview.candidates.length" type="button" class="clean" :disabled="!!busy" @click="clean">{{ t(busy === 'clean' ? 'storage.cleaning' : 'storage.clean') }} · {{ size(preview.reclaimableBytes) }}</button>
        </div>
      </template>
    </div>
  </details>
</template>

<script setup lang="ts">
import { computed, onUnmounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { HardDrive, RefreshCw, ShieldCheck } from 'lucide-vue-next'
import type { StoragePreview, StorageResult } from '../../shared/ipc-contract.ts'
import { electronAPI } from '../native/electron-api.ts'
const { t, locale } = useI18n()
const open = ref(false), busy = ref(''), error = ref(''), message = ref('')
const preview = ref<StoragePreview>()
let revision = 0
const total = computed(() => preview.value?.categories.reduce((sum, category) => sum + category.bytes, 0) || 0)
function size(bytes: number) {
  const units = ['B', 'KiB', 'MiB', 'GiB']; let index = 0
  while (bytes >= 1024 && index < units.length - 1) { bytes /= 1024; index++ }
  return `${new Intl.NumberFormat(locale.value, { maximumFractionDigits: index ? 1 : 0 }).format(bytes)} ${units[index]}`
}
async function perform(action: 'scan' | 'clean') {
  if (busy.value) return
  const id = preview.value?.id
  if (action === 'clean' && !id) return
  const run = ++revision
  busy.value = action; error.value = ''; message.value = ''; preview.value = undefined
  try {
    const result: StorageResult = action === 'scan' ? await electronAPI.storage.preview() : await electronAPI.storage.clean(id!)
    if (run !== revision) return
    if (!result.success) {
      error.value = result.issue
      if (result.removedFiles) message.value = t('storage.partial', { count: result.removedFiles, size: size(result.removedBytes || 0) })
    } else if (result.preview) preview.value = result.preview
    else message.value = t('storage.done', { count: result.removedFiles || 0, size: size(result.removedBytes || 0) })
  } catch { if (run === revision) error.value = action === 'scan' ? 'scan-failed' : 'cleanup-failed' }
  finally { if (run === revision) busy.value = '' }
}
const scan = () => perform('scan'), clean = () => perform('clean')
function toggle(event: Event) {
  if (event.target !== event.currentTarget) return
  open.value = (event.currentTarget as HTMLDetailsElement).open
  if (open.value) void scan()
  else { revision++; preview.value = undefined; busy.value = ''; error.value = ''; message.value = '' }
}
onUnmounted(() => { revision++ })
</script>

<style scoped>
.storage-panel{margin:0 0 18px;border:1px solid #354653;border-radius:12px;background:linear-gradient(130deg,#192933,#131e2c);color:#e3eaf0;font-size:13px}.storage-panel summary{display:flex;gap:14px;align-items:center;padding:20px;cursor:pointer;list-style:none}.storage-panel summary::-webkit-details-marker{display:none}.storage-panel summary>svg{color:#c5ad77}.storage-panel summary strong{font-size:15px;font-weight:600}.storage-panel small{display:block;margin-top:5px;color:#9caec0;font-size:11px}.storage-body{padding:0 20px 20px}.storage-body p{margin:0;line-height:1.7}.muted{color:#a4b5c5;font-size:12px}.protected{display:flex;align-items:flex-start;gap:10px;padding:13px;margin:16px 0;border:1px solid #355955;border-radius:8px;color:#a5d8cc;background:#152f30;font-size:12px}.protected svg{flex:none;margin-top:2px}.toolbar{display:flex;justify-content:flex-end;margin:12px 0}button{display:inline-flex;align-items:center;justify-content:center;gap:8px;max-width:100%;border:1px solid #456170;border-radius:7px;background:#253a49;color:#deedf4;padding:10px 14px;font:inherit;cursor:pointer}button:disabled{opacity:.55;cursor:wait}button:focus-visible,summary:focus-visible{outline:2px solid #7ddeca;outline-offset:4px}.totals{display:grid;grid-template-columns:1fr 1fr;gap:16px;padding:17px 0}.totals strong{display:block;font-size:25px;font-weight:500;font-variant-numeric:tabular-nums;margin-top:8px}.totals>div:last-child strong{color:#dfc990}.categories,.candidates{list-style:none;padding:0;margin:0}.categories li,.candidates li{display:flex;justify-content:space-between;align-items:center;gap:18px;padding:12px 0;border-bottom:1px solid #2b3d4c}.categories strong,.candidates strong{white-space:nowrap;font-size:12px;font-weight:500}.cleanup-preview{margin-top:22px;padding:16px;border:1px solid #5b5039;border-radius:9px;background:#1b242c}.cleanup-preview h3{font-size:14px;margin:0 0 9px;color:#e0ca97}.candidates{max-height:260px;overflow:auto;margin:10px 0 17px}.candidates span{overflow-wrap:anywhere}.clean{background:#c7b17b;border-color:#ddca98;color:#1b262c;font-weight:600;white-space:normal}.empty{padding-top:15px;color:#a7d9ce}.warning{color:#edbd91;margin:12px 0!important;font-size:12px}.notice{padding:12px;background:#193b36;border-radius:7px;margin:12px 0!important}@media(max-width:540px){.storage-body{padding:0 14px 16px}.storage-panel summary{padding:16px}.totals strong{font-size:21px}.cleanup-preview{padding:12px}.clean{width:100%}}
</style>
