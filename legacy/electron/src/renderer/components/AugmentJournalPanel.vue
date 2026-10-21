<template>
  <details class="journal" @toggle="onToggle">
    <summary>{{ t('matchHistory.journal.title') }}</summary>
    <div v-if="open" class="journal-body">
      <div class="journal-toolbar">
        <p>{{ t('matchHistory.journal.description') }}</p>
        <button type="button" :disabled="loading" @click="refresh">{{ t('matchHistory.journal.refresh') }}</button>
      </div>
      <p v-if="loading" role="status">{{ t('matchHistory.journal.loading') }}</p>
      <p v-if="error" role="alert">{{ error }}</p>
      <template v-if="data">
        <p v-if="data.offline" class="notice">{{ t('matchHistory.journal.offline') }}</p>
        <p v-if="data.playerName" class="journal-account">{{ data.playerName }} · {{ data.platformId }}</p>
        <p v-if="!data.sessions.length">{{ t('matchHistory.journal.empty') }}</p>
        <div v-if="data.sessions.length" class="review-filters">
          <label>{{ t('matchHistory.journal.recap.champion') }}<select v-model="championFilter"><option value="">{{ t('matchHistory.journal.recap.allChampions') }}</option><option v-for="champion in champions" :key="champion.id" :value="String(champion.id)">{{ champion.name }}</option></select></label>
          <label>{{ t('matchHistory.journal.recap.result') }}<select v-model="resultFilter"><option value="">{{ t('matchHistory.journal.recap.allResults') }}</option><option v-for="result in ['win', 'loss', 'remake', 'unknown']" :key="result" :value="result">{{ t(`matchHistory.journal.recap.${result}`) }}</option></select></label>
          <p>{{ t('matchHistory.journal.recap.journalScope') }}</p>
        </div>
        <p v-if="data.sessions.length && !filteredSessions.length">{{ t('matchHistory.journal.recap.noMatches') }}</p>
        <details v-for="session in filteredSessions" :key="session.gameId" class="journal-session" @toggle="toggleSession($event, session.gameId)">
          <summary>
            <span>{{ formatTime(session.startedAt) }}</span>
            <span>{{ session.championId ? session.championName || t('matchHistory.championFallback', { id: session.championId }) : t('matchHistory.journal.recap.unknown') }}<template v-if="session.result"> · {{ t(`matchHistory.journal.recap.${session.result}`) }}</template></span>
            <span>{{ t('matchHistory.journal.entries', { count: session.events.length }) }}</span>
            <small>{{ t(`matchHistory.journal.${session.interrupted ? 'incomplete' : session.endedAt ? 'finished' : 'active'}`) }}</small>
          </summary>
          <template v-if="expanded.has(session.gameId)">
          <RecognitionRecap :summary="session.recognition" />
          <p class="notice">{{ t(`matchHistory.journal.${session.finalAugmentIds?.length ? 'finalHint' : 'pending'}`) }}</p>
          <p v-if="session.capped" class="notice">{{ t('matchHistory.journal.capped') }}</p>
          <ol class="journal-events">
            <li v-for="(event, index) in session.events" :key="index" class="journal-event">
              <div class="journal-event-meta">
                <time>{{ formatTime(event.observedAt) }}</time>
                <strong>{{ t(`matchHistory.journal.${event.kind}`) }}</strong>
                <span>{{ event.level === null ? t('matchHistory.journal.unknownLevel') : t('matchHistory.journal.level', { level: event.level }) }}</span>
                <small v-if="event.partial">{{ t('matchHistory.journal.partial') }}</small>
              </div>
              <div class="journal-cards">
                <div v-for="(card, slot) in event.cards" :key="slot" class="journal-card" :class="{ missing: !card }">
                  <img v-if="card?.iconUrl" :src="card.iconUrl" alt="" />
                  <span v-else class="journal-placeholder" aria-hidden="true">{{ card ? '◇' : '—' }}</span>
                  <span>{{ card ? card.name || t('matchHistory.journal.unknown', { id: card.id }) : t('matchHistory.journal.missing') }}</span>
                  <small v-if="card && session.finalAugmentIds?.includes(card.id)" class="final-badge">{{ t('matchHistory.journal.final') }}</small>
                </div>
              </div>
            </li>
          </ol>
          </template>
        </details>
      </template>
    </div>
  </details>
</template>

<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import RecognitionRecap from './RecognitionRecap.vue'
import { useI18n } from 'vue-i18n'
import type { AugmentJournalResult } from '../../shared/ipc-contract.ts'
import { electronAPI, hasElectronAPI } from '../native/electron-api.ts'

const { t, locale } = useI18n()
const open = ref(false)
const loading = ref(false)
const error = ref('')
const data = ref<AugmentJournalResult['data']>()
const expanded = ref(new Set<number>())
const championFilter = ref('')
const resultFilter = ref('')
const champions = computed(() => [...new Map((data.value?.sessions || []).filter(s => s.championId).map(s => [s.championId!, { id: s.championId!, name: s.championName || t('matchHistory.championFallback', { id: s.championId }) }])).values()])
const filteredSessions = computed(() => (data.value?.sessions || []).filter(s => (!championFilter.value || String(s.championId) === championFilter.value) && (!resultFilter.value || (s.result || 'unknown') === resultFilter.value)))
let revision = 0
let unsubscribe: (() => void) | undefined
const formatTime = (value: number) => new Intl.DateTimeFormat(locale.value, {
  month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit', second: '2-digit',
}).format(new Date(value))

async function refresh() {
  const current = ++revision
  loading.value = true
  error.value = ''
  data.value = undefined
  championFilter.value = ''; resultFilter.value = ''
  expanded.value.clear()
  try {
    const result = await electronAPI.matchHistory.getSelectionJournal()
    if (current !== revision) return
    if (result.success) data.value = result.data
    else error.value = ['cache-unavailable', 'cache-write-failed'].includes(result.issue || '')
      ? t('matchHistory.journal.failed') : t(`matchHistory.connection.issues.${result.issue || 'service-unavailable'}`)
  } catch { if (current === revision) error.value = t('matchHistory.journal.failed') }
  finally { if (current === revision) loading.value = false }
}
function onToggle(event: Event) {
  // Nested session details also emit toggle in some DOM implementations.
  if (event.target !== event.currentTarget) return
  const nextOpen = (event.target as HTMLDetailsElement).open
  if (nextOpen === open.value) return
  open.value = nextOpen
  if (open.value && hasElectronAPI()) void refresh()
  else { revision++; loading.value = false }
}
function toggleSession(event: Event, gameId: number) {
  if (event.target !== event.currentTarget) return
  if ((event.target as HTMLDetailsElement).open) expanded.value.add(gameId)
  else expanded.value.delete(gameId)
}
watch(locale, () => { if (open.value) void refresh() })
onMounted(() => {
  if (hasElectronAPI()) unsubscribe = electronAPI.events.on('match-history-updated', () => {
    if (open.value && !loading.value) void refresh()
  })
})
onUnmounted(() => { revision++; unsubscribe?.() })
</script>

<style scoped>
.review-filters { display: flex; flex-wrap: wrap; gap: 8px 12px; margin: 12px 0; }
.review-filters label { display: flex; flex: 1 1 160px; flex-direction: column; gap: 5px; font-size: 12px; }
.review-filters select { min-width: 0; padding: 9px; border: 1px solid #58615a; border-radius: 4px; background: #14232d; color: #dce7ed; }
.review-filters p { flex-basis: 100%; margin: 0; color: #a3b1b9; }
.journal { margin-top: 12px; border: 1px solid #40505a; border-radius: 6px; background: #0a1720; color: #c6d0d5; }
summary { padding: 12px; cursor: pointer; color: #dbc38f; font-size: 14px; font-weight: 650; }
summary:focus-visible, button:focus-visible { outline: 2px solid #c8aa6e; outline-offset: 2px; }
.journal-body { padding: 0 12px 12px; }
p { font-size: 12px; line-height: 1.7; margin: 8px 0; }
.journal-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.journal-toolbar p { flex: 1 1 280px; }
button { padding: 9px 12px; min-height: 40px; border: 1px solid #665936; border-radius: 4px; background: #202b30; color: #e2c791; cursor: pointer; font-size: 12px; }
button:disabled { opacity: .5; cursor: default; }
.journal-account { color: #dce7ed; overflow-wrap: anywhere; }
.journal-session { margin-top: 10px; border: 1px solid #293d49; border-radius: 4px; }
.journal-session summary span, .journal-session summary small { margin-right: 14px; }
.journal-session summary small { color: #a3b1b9; white-space: nowrap; display: inline-block; }
.notice { color: #d4bd8d; padding: 0 10px; }
.journal-events { list-style: none; padding: 0 10px 10px; margin: 0; }
.journal-event { padding: 12px 0; border-top: 1px solid #273b45; }
.journal-event-meta { display: flex; flex-wrap: wrap; gap: 6px 12px; font-size: 12px; line-height: 1.7; margin-bottom: 10px; }
.journal-event-meta time { color: #a3b1b9; }
.journal-cards { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 8px; }
.journal-card { display: flex; flex-direction: column; align-items: center; gap: 7px; min-width: 0; padding: 10px 6px; border: 1px solid #415057; border-radius: 4px; background: #14232d; text-align: center; font-size: 12px; overflow-wrap: anywhere; }
.journal-card img, .journal-placeholder { width: 42px; height: 42px; object-fit: contain; }
.journal-placeholder { display: grid; place-items: center; color: #bda36d; font-size: 26px; }
.journal-card.missing { opacity: .65; border-style: dashed; }
.final-badge { color: #e5ca8c; border: 1px solid #766340; padding: 2px 5px; border-radius: 3px; font-size: 11px; }
</style>
