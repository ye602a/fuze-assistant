<template>
  <section class="update-panel" aria-labelledby="update-heading">
    <div class="update-heading"><div><small>FUZE / RELEASES</small><h2 id="update-heading">{{ t('title') }}</h2></div><span class="channel">{{ t('private') }}</span></div>
    <div class="release-line"><strong>v{{ state?.currentVersion || '—' }}</strong><span v-if="state?.latestVersion">→ v{{ state.latestVersion }}</span><span class="phase" role="status">{{ t('phase_' + (state?.phase || 'disabled')) }}</span></div>
    <p>{{ t('policy') }}</p>
    <p v-if="state?.gamePhase && ['ChampSelect', 'GameStart', 'InProgress', 'Reconnect'].includes(String(state.gamePhase))" class="message">{{ t('game') }}</p>
    <p v-if="!state?.installationSupported" class="message">{{ t('portable') }}</p>
    <p v-if="errorCode" class="error" role="alert">{{ t('error_' + errorCode) }}</p>
    <progress v-if="state?.phase === 'downloading'" :value="state.progress?.percent || 0" max="100" :aria-label="t('download')" />
    <div class="actions">
      <button :disabled="pending || !state?.canCheck" @click="run(() => electronAPI.appUpdate.check())">{{ t('check') }}</button>
      <button v-if="state?.available && !state?.downloaded && state?.installationSupported" class="primary" :disabled="pending || !state?.canDownload" @click="run(() => electronAPI.appUpdate.download())">{{ t('download') }}</button>
      <button v-if="state?.downloaded" class="primary" :disabled="pending || !state?.canInstall" @click="confirmInstall = true">{{ t('install') }}</button>
      <button class="link" @click="electronAPI.shell.openExternal(releaseUrl)">{{ t('releases') }} ↗</button>
    </div>
    <details class="authorization"><summary>{{ t(state?.credentialConfigured ? 'authorized' : 'authorize') }}</summary><p>{{ t('authHint') }}</p><form @submit.prevent="saveCredential"><label for="update-token">{{ t('token') }}</label><div class="credential-row"><input id="update-token" v-model="token" type="password" autocomplete="off" spellcheck="false" maxlength="512" :placeholder="t('tokenPlaceholder')"/><button :disabled="pending || !token.trim()">{{ t('save') }}</button><button v-if="state?.credentialConfigured" type="button" :disabled="pending" @click="run(() => electronAPI.appUpdate.setCredential(''))">{{ t('forget') }}</button></div></form><p>{{ t('encrypted') }}</p></details>
    <div v-if="confirmInstall" class="confirm" role="alertdialog" aria-modal="true" :aria-label="t('install')"><p>{{ t('confirm') }}</p><div class="actions"><button autofocus @click="confirmInstall = false">{{ t('cancel') }}</button><button class="primary" :disabled="pending || !state?.canInstall" @click="install">{{ t('confirmButton') }}</button></div></div>
  </section>
</template>
<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { electronAPI } from '../native/electron-api.ts'
import { updateMessages } from '../i18n/update.ts'
import type { AppUpdateState, OperationResult } from '../../shared/ipc-contract.ts'
const { t } = useI18n({ useScope: 'local', messages: updateMessages })
const state = ref<AppUpdateState | null>(null)
const token = ref('')
const pending = ref(false)
const actionError = ref('')
const confirmInstall = ref(false)
const releaseUrl = 'https://github.com/ye602a/fuze-assistant/releases'
const errorCode = computed(() => actionError.value || String(state.value?.error || ''))
let unsubscribe: (() => void) | undefined
async function run(action: () => Promise<OperationResult & { data?: AppUpdateState }>) {
  if (pending.value) return
  pending.value = true; actionError.value = ''
  try { const result = await action(); if (!result.success) actionError.value = result.error || 'network' }
  catch { actionError.value = 'network' }
  finally {
    try { state.value = (await electronAPI.appUpdate.getState()).data || null } catch { /* Keep last known state. */ }
    pending.value = false
  }
}
async function saveCredential() { const value = token.value.trim(); token.value = ''; await run(() => electronAPI.appUpdate.setCredential(value)) }
async function install() { confirmInstall.value = false; await run(() => electronAPI.appUpdate.install()) }
onMounted(async () => { unsubscribe = electronAPI.events.on('app-update-status-changed', value => { state.value = value }); await run(() => electronAPI.appUpdate.getState()) })
onBeforeUnmount(() => { token.value = ''; unsubscribe?.() })
</script>
<style scoped>
.update-panel{position:relative;background:#151d2b;border:1px solid #283244;border-radius:13px;padding:22px;color:#e9edf4;margin-bottom:18px}.update-heading{display:flex;justify-content:space-between;gap:12px;align-items:center}.update-heading small{font-size:10px;letter-spacing:1.5px;color:#70ddcc}.update-heading h2{font-size:17px;margin:6px 0 16px}.channel{border:1px solid #43506a;border-radius:20px;padding:4px 9px;font-size:11px;color:#afc2de}.release-line{display:flex;gap:12px;align-items:center;flex-wrap:wrap}.release-line strong{font-size:21px;letter-spacing:-.5px}.phase{color:#8fe4d3;font-size:12px}.update-panel p{font-size:12px;line-height:1.8;color:#9eafc7;margin:10px 0}.actions{display:flex;gap:8px;flex-wrap:wrap;margin:16px 0 4px}button{font:inherit;border:1px solid #3a4b62;border-radius:7px;background:#253247;color:#dbe5f3;cursor:pointer;padding:9px 13px;font-size:12px}button:disabled{opacity:.4;cursor:default}button.primary{color:#102c2b;background:#79decc;border-color:#79decc;font-weight:600}button.link{background:transparent;border-color:transparent;color:#a8bdd9}.authorization{margin-top:18px;border-top:1px solid #283244;padding-top:16px}.authorization summary{cursor:pointer;font-size:12px;color:#b4c5d9}.credential-row{display:flex;gap:8px;margin-top:7px}.credential-row input{flex:1;min-width:0;font:inherit;padding:10px;border:1px solid #394960;border-radius:6px;background:#0e1623;color:#e9edf4}.authorization label{font-size:11px;color:#b4c5d9}.update-panel .error{color:#f3acaf}.update-panel .message{color:#d2bd92}progress{width:100%;height:6px;accent-color:#79decc}.confirm{padding:16px;border:1px solid #52716d;border-radius:10px;background:#172e33;margin-top:16px}.confirm p{color:#d4e3e4}button:focus-visible,input:focus-visible,summary:focus-visible{outline:2px solid #79decc;outline-offset:3px}
</style>
