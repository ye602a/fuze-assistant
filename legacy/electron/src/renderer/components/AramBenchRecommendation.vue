<template>
  <section class="aram-panel" :class="{ compact }">
    <header class="panel-header"><div class="title-lockup"><Swords class="panel-icon"/><h3>{{ t('bench.title') }}</h3></div><div class="panel-actions"><span class="status-pill" :class="statusClass">{{ statusLabel }}</span><button class="refresh-btn" type="button" :disabled="loading" :aria-label="t('bench.refreshTitle')" @click="refresh"><RefreshCw class="panel-icon" :class="{spinning: loading}"/></button></div></header>
    <div v-if="loading && !recommendation" class="empty-state"><LoaderCircle class="panel-icon spinning"/>{{ t('bench.reading') }}</div>
    <div v-else-if="error" class="empty-state error-state"><CircleAlert class="panel-icon"/>{{ error }}</div>
    <div v-else-if="!isChampSelect" class="empty-state"><CircleDashed class="panel-icon"/>{{ emptyMessage }}</div>
    <div v-else class="recommendation-body">
      <h4>{{ t('bench.ourTeam') }}</h4>
      <div class="team-line" :aria-label="t('bench.ourTeam')">
        <article v-for="member in teamSlots" :key="member.slot" class="team-member" :class="{self:member.isCurrent}" :title="member.champion?.name || t('bench.unselected')">
          <div class="team-portrait"><img v-if="member.champion?.iconUrl" :src="member.champion.iconUrl" alt=""/><span v-else>◇</span><small v-if="member.isCurrent">{{ t('bench.you') }}</small></div>
          <strong>{{ member.champion?.name || t('bench.unselected') }}</strong><span>{{ formatPercent(member.champion?.winRate) }}</span>
        </article>
      </div>
      <div class="candidate-heading"><h4>{{ t('bench.availableRank') }}</h4><span>{{ t('bench.byWinRate') }}</span></div>
      <p class="bench-note">{{ t('bench.teamReference') }}</p>
      <div class="candidate-grid" role="region" :aria-label="t('bench.candidateList')">
        <article v-for="candidate in topCandidates" :key="candidate.championId" class="candidate-card" :class="{recommended:isRecommended(candidate)}">
          <div class="champion-mark"><img v-if="candidate.iconUrl" :src="candidate.iconUrl" alt=""/><span v-else>◇</span></div>
          <div class="candidate-main"><strong>{{ candidate.name }}</strong></div>
          <div class="candidate-win"><small>{{ t('bench.winRateLabel') }}</small><strong>{{ formatPercent(candidate.winRate) }}</strong></div>
          <small class="candidate-pick">{{ t('bench.pickRate', {value:formatPercent(candidate.pickRate)}) }}</small>
        </article>
        <p v-if="!topCandidates.length" class="empty-state">{{ t('bench.noAvailable') }}</p>
      </div>
    </div>
  </section>
</template>
<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import {
    CircleAlert,
    CircleDashed,
    LoaderCircle,
    RefreshCw,
    Swords,
} from 'lucide-vue-next'
import { electronAPI, hasElectronAPI } from '../native/electron-api.ts'
import { useI18n } from 'vue-i18n'

const props = defineProps({
    compact: {
        type: Boolean,
        default: false,
    },
    previewRecommendation: {
        type: Object,
        default: null,
    },
})

const { t } = useI18n()

const recommendation = ref(null)
const loading = ref(false)
const error = ref('')
const refreshTimer = ref(null)
const previewMode = ref(false)
const requestInFlight = ref(false)
const unsubscribeEvents = []
const BENCH_REFRESH_TIMEOUT_MS = 8 * 1000
let mounted = false
let revision = 0
let refreshQueued = false

const isChampSelect = computed(() => recommendation.value?.gameflowPhase === 'ChampSelect')

const statusClass = computed(() => {
    const status = recommendation.value?.status
    if (loading.value) return 'loading'
    if (status === 'ready') return 'ready'
    if (status === 'no-bench') return 'muted'
    if (status === 'no-current-champion') return 'warning'
    return 'idle'
})

const statusLabel = computed(() => {
    if (loading.value) return t('bench.refreshing')

    const status = recommendation.value?.status
    if (status === 'ready') return t('bench.readOnly')
    if (status === 'no-bench') return t('bench.noBench')
    if (status === 'no-current-champion') return t('bench.noChampion')
    if (status === 'no-candidates') return t('bench.noCandidates')
    return t('bench.waitingSelection')
})

const emptyMessage = computed(() => {
    if (!hasElectronAPI()) return t('bench.apiUnavailable')
    if (recommendation.value?.reason === 'lcu-unavailable') return t('bench.waitingClient')
    return t('bench.waitingChampSelect')
})

const topCandidates = computed(() => (recommendation.value?.candidates || [])
    .filter(candidate => candidate.source === 'bench')
    .sort((a, b) => (b.winRate ?? -1) - (a.winRate ?? -1) || a.championId - b.championId))
const teamSlots = computed(() => Array.from({length: 5}, (_, slot) => recommendation.value?.team?.[slot] || {slot, champion: slot === 0 ? recommendation.value?.currentChampion : null, isCurrent: slot === 0}))
const recommended = computed(() => topCandidates.value.find(candidate => candidate.winRate != null) || null)

const withTimeout = (promise, timeoutMs, message) => {
    let timeoutId
    const timeout = new Promise((_, reject) => {
        timeoutId = setTimeout(() => reject(new Error(message)), timeoutMs)
    })

    return Promise.race([promise, timeout]).finally(() => clearTimeout(timeoutId))
}

const applyPreviewRecommendation = (data) => {
    if (!data) {
        return
    }

    revision += 1
    previewMode.value = data.refreshable !== true
    recommendation.value = data
    error.value = ''
    loading.value = false

    if (data.refreshable === true && mounted) {
        refreshQueued = requestInFlight.value
        refresh(false)
    }
}

const refresh = async (showLoading = true) => {
    if (document.hidden && !showLoading) return
    if (!hasElectronAPI()) {
        error.value = t('bench.apiUnavailable')
        return
    }

    if (previewMode.value && !showLoading) {
        return
    }

    if (previewMode.value && showLoading) {
        previewMode.value = false
    }

    if (requestInFlight.value) {
        return
    }

    const requestRevision = revision
    requestInFlight.value = true
    if (showLoading) {
        loading.value = true
    }
    error.value = ''

    try {
        const result = await withTimeout(
            electronAPI.lcu.getAramBenchRecommendation(),
            BENCH_REFRESH_TIMEOUT_MS,
            t('bench.timeout', { seconds: BENCH_REFRESH_TIMEOUT_MS / 1000 })
        )
        if (!mounted || previewMode.value || revision !== requestRevision) {
            return
        }

        if (result?.success && result.recommendation) {
            recommendation.value = result.recommendation
        } else {
            error.value = result?.error || t('bench.loadFailed')
        }
    } catch (err) {
        if (!mounted || previewMode.value || revision !== requestRevision) {
            return
        }

        error.value = err?.message || t('bench.loadFailed')
    } finally {
        requestInFlight.value = false
        loading.value = false
        if (refreshQueued && mounted && !previewMode.value) {
            refreshQueued = false
            void refresh(false)
        }
    }
}

const isRecommended = (candidate) =>
    recommended.value && candidate.championId === recommended.value.championId

const formatPercent = (value) => {
    if (value == null || Number.isNaN(Number(value))) return '--'
    return `${(Number(value) * 100).toFixed(1)}%`
}

watch(
    () => props.previewRecommendation,
    (data) => {
        if (data) {
            applyPreviewRecommendation(data)
            return
        }

        revision += 1
        previewMode.value = false
        recommendation.value = null
        error.value = ''
        if (mounted) {
            refreshQueued = requestInFlight.value
            refresh(false)
        }
    },
    { immediate: true }
)

onMounted(() => {
    mounted = true

    if (hasElectronAPI()) {
        unsubscribeEvents.push(electronAPI.events.on('bench-recommendation-preview', (data) => {
            applyPreviewRecommendation(data)
        }))

        unsubscribeEvents.push(electronAPI.events.on('game-phase-changed', () => {
            revision += 1
            previewMode.value = false
            recommendation.value = null
            error.value = ''
            refreshQueued = requestInFlight.value
            void refresh(false)
        }))
    }

    if (!previewMode.value) {
        refresh()
    }
    refreshTimer.value = setInterval(() => refresh(false), 3000)

})

onBeforeUnmount(() => {
    if (refreshTimer.value) {
        clearInterval(refreshTimer.value)
        refreshTimer.value = null
    }

    unsubscribeEvents.splice(0).forEach((unsubscribe) => unsubscribe())
    mounted = false
    revision += 1
    refreshQueued = false
})
</script>
<style scoped>

.aram-panel { padding: 10px; background: linear-gradient(145deg,#142837,#091723); border: 1px solid #635637; color: #f0e6d2; font-family: 'Segoe UI','Microsoft YaHei',sans-serif; }
.panel-header,.title-lockup,.panel-actions,.candidate-heading { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
.panel-header { margin-bottom: 10px; }
.title-lockup { justify-content: flex-start; }
h3,h4,p { margin: 0; }
h3 { font-size: 13px; }
h4 { font-size: 11px; color: #c8aa6e; font-weight: 600; }
.panel-icon { width: 16px; height: 16px; flex-shrink: 0; }
.panel-actions { gap: 5px; }
.status-pill { font-size: 10px; color: #b0c5cc; }
.refresh-btn { width: 25px; height: 25px; display: grid; place-items: center; color: #cdbe91; background: #0a1926; border: 1px solid #43525a; cursor: pointer; }
.refresh-btn:disabled { opacity: .4; }
.team-line { display: grid; grid-template-columns: repeat(5,minmax(0,1fr)); gap: 5px; margin: 7px 0 12px; padding: 8px 4px; border: 1px solid #35566a; background: #0b2030; }
.team-member { min-width: 0; text-align: center; }
.team-portrait { width: 100%; max-width: 36px; aspect-ratio: 1; margin: auto; position: relative; display: grid; place-items: center; border: 1px solid #547b91; background: #07121c; color: #6c8b9a; }
.team-portrait img,.champion-mark img { width: 100%; height: 100%; object-fit: cover; }
.team-portrait small { position: absolute; bottom: 0; right: 0; font-size: 9px; padding: 0 3px; color: #06171b; background: #91cadc; }
.team-member.self .team-portrait { border-color: #9fe3ed; box-shadow: 0 0 6px #44879755; }
.team-member>strong { display: block; overflow-wrap: anywhere; font-size: 10px; font-weight: 500; line-height: 1.25; margin: 4px 0; }
.team-member>span { font-size: 10px; color: #91bbc9; font-variant-numeric: tabular-nums; }
.candidate-heading>span,.bench-note { font-size: 10px; color: #91a6b1; }
.bench-note { line-height: 1.4; margin: 5px 0 7px; }
.candidate-grid { display: grid; grid-template-columns: repeat(auto-fit,minmax(78px,1fr)); gap: 5px; }
.candidate-card { min-width: 0; display: grid; grid-template-columns: 24px minmax(0,1fr); align-content: start; align-items: center; gap: 4px; padding: 6px; border: 1px solid #62583e; background: linear-gradient(150deg,#272b27,#142028); }
.candidate-card.recommended { border-color: #c8aa6e; background: linear-gradient(150deg,#3c3526,#19282d); }
.champion-mark { width: 24px; height: 24px; display: grid; place-items: center; border: 1px solid #8b7851; }
.candidate-main { min-width: 0; }
.candidate-main strong { display: block; font-size: 11px; line-height: 1.25; overflow-wrap: anywhere; font-weight: 500; }
.candidate-win { grid-column: 1 / -1; display: flex; flex-wrap: wrap; align-items: baseline; justify-content: space-between; gap: 2px; }
.candidate-win small,.candidate-pick { color: #acb9ba; font-size: 10px; line-height: 1.3; }
.candidate-win strong { font-size: 17px; color: #f0cf88; font-variant-numeric: tabular-nums; }
.candidate-pick { grid-column: 1 / -1; }
.empty-state { display: flex; align-items: center; gap: 8px; padding: 12px 0; font-size: 11px; color: #9fb1bc; line-height: 1.6; }
.candidate-grid>.empty-state { grid-column: 1 / -1; }
.error-state { color: #f19b9b; }
button:focus-visible { outline: 2px solid #69bce6; outline-offset: 2px; }
.spinning { animation: spin 1s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
</style>
