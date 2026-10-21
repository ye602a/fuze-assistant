<template>
  <transition name="float-fade">
    <div v-if="visible" class="floating-overlay">
      <div class="floating-header"><span class="floating-title">FUZE <small>{{ t('augment.recommendation') }}</small></span><span class="statistics-basis">{{ t('workbench.overallBasis') }}</span><button class="close-btn" type="button" @click="closeOverlay('manual')" :title="t('common.close')" :aria-label="t('common.close')">
        <X class="close-icon" />
      </button></div>


      <!-- 加载状态 -->
      <div v-if="loading" class="loading">
        <div class="spinner"></div>
        <span>{{ t('common.loading') }}</span>
      </div>

      <!-- 海克斯推荐列表 -->
      <div v-else-if="previewAugments.length > 0" class="augments-grid">
        <div
          v-for="(augment, index) in previewAugments"
          :key="getAugmentKey(augment, index)"
          class="augment-item"
          :class="[`rarity-${augment.rarity || 'unknown'}`, { 'top-pick': !augment.missing && isTopPick(augment, index), 'empty-slot': augment.missing }]"
        >
          <div v-if="!augment.missing && isTopPick(augment, index)" class="top-pick-badge">
            <span>↗</span>
            {{ t('augment.priority') }}
          </div>

          <div class="augment-icon-frame">
            <GameIcon
              v-if="!augment.missing"
              :src="augment.iconPath ? getAugmentIconUrl(augment.iconPath) : ''"
              :alt="augment.name"
              class="augment-icon"
            />
          </div>

          <div class="content">
            <h3 class="name" :title="augment.missing ? '' : augment.name">{{ augment.missing ? t('workbench.missing') : augment.name }}</h3>
            <p v-if="augment.missing" class="missing-hint">{{ t('workbench.missingHint') }}</p>
            <div v-if="!augment.missing" class="stats-lines">
              <div class="stat-line">
                <span>{{ t('augment.pickRateShort') }}</span>
                <strong>{{ formatPercent(augment.pickRate) }}</strong>
              </div>
              <div class="stat-line">
                <span>{{ t('augment.winRate') }}</span>
                <strong :title="formatAugmentWinRate(augment.winRate)">{{ augment.winRate == null || augment.winRate === '' ? '—' : formatAugmentWinRate(augment.winRate) }}</strong>
              </div>
            </div>
          </div>

          <div v-if="!augment.missing" class="recommendation">
            <div class="recommend-label"><span>{{ t('augment.recommendation') }}</span><strong>{{ getRecommendText(augment.recommendScore) }} · {{ formatScore(augment.recommendScore) }}</strong></div>
            <div class="score-track" role="meter" :aria-label="t('augment.recommendation')" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="augment.recommendScore == null ? undefined : Math.min(100, Math.max(0, Math.round(augment.recommendScore * 100)))" :aria-valuetext="getRecommendText(augment.recommendScore)">
              <div class="score-fill" :style="{ width: getScoreWidth(augment.recommendScore) }"></div>
            </div>
          </div>
        </div>
      </div>

      <!-- 错误状态 -->
      <div v-else-if="error" class="error">
        {{ error }}
      </div>
    </div>
  </transition>
</template>

<script setup>
import { computed, ref, onMounted, onBeforeUnmount } from 'vue'
import { X } from 'lucide-vue-next'
import GameIcon from './GameIcon.vue'
import { electronAPI } from '../native/electron-api.ts'
import { sortAugmentsByDetectedOrder } from '../service/augment-order.js'
import {
  getAugmentKey,
  getTopPickKey,
  mergeWinrateWithDetectedSlots
} from '../service/augment-display.js'
import { getAugmentIconUrl } from '../service/cdn'
import { formatAugmentWinRate, formatPercent } from '../service/overlay-formatters.ts'
import { useI18n } from 'vue-i18n'

const { t } = useI18n()
const visible = ref(false)
const loading = ref(false)
const error = ref(null)
const displayAugments = ref([])
const championId = ref(null)
const unsubscribeEvents = []
let overlayRequestId = 0

const previewAugments = computed(() => displayAugments.value.slice(0, 3))

const topPickKey = computed(() => getTopPickKey(previewAugments.value))

const isTopPick = (augment, index) => getAugmentKey(augment, index) === topPickKey.value

/**
 * 格式化分数
 */
const formatScore = (value) => {
  if (value == null || isNaN(value)) return '--'
  return Math.round(value * 100)
}

const getScoreWidth = (value) => {
  if (value == null || isNaN(value)) return '0%'
  return `${Math.min(Math.max(value * 100, 0), 100)}%`
}

/**
 * 获取推荐文本
 */
const getRecommendText = (score) => {
  if (score == null || isNaN(score)) return t('augment.scoreUnknown')
  if (score >= 0.6) return t('augment.scoreMustPick')
  if (score >= 0.5) return t('augment.scoreStrong')
  if (score >= 0.4) return t('augment.scoreRecommended')
  if (score >= 0.3) return t('augment.scoreOptional')
  return t('augment.scoreNiche')
}

const logFloatingInfo = (message, details = {}) => {
  try {
    electronAPI.diagnostics.logRendererInfo({
      type: 'augment-floating',
      source: 'AugmentFloatingOverlay',
      message,
      url: window.location.href,
      details,
      timestamp: Date.now(),
    })
  } catch {
    // Best-effort diagnostics only.
  }
}

const getAugmentDebugId = (augment) => augment?.id ?? augment?.augmentId ?? null

const getTopPickDiagnostics = () => {
  const key = topPickKey.value
  const index = previewAugments.value.findIndex((augment, slot) => getAugmentKey(augment, slot) === key)
  const augment = index >= 0 ? previewAugments.value[index] : null

  return {
    key,
    index,
    id: getAugmentDebugId(augment),
    name: augment?.name || null,
    recommendScore: augment?.recommendScore ?? null,
  }
}

const getDisplayDiagnostics = () => ({
  augmentIds: previewAugments.value.map(getAugmentDebugId),
  detectedSlots: previewAugments.value.map((augment, index) => (
    Number.isInteger(augment?.detectedSlot) ? augment.detectedSlot : index
  )),
  missingSlots: previewAugments.value
    .map((augment, index) => augment?.missing ? index : null)
    .filter(index => index != null),
  topPick: getTopPickDiagnostics(),
})

const logDisplayState = (message, data, details = {}) => {
  logFloatingInfo(message, {
    ...details,
    championId: championId.value || null,
    requestAugmentIds: (data?.augments || []).map(getAugmentDebugId),
    requestDiagnostics: data?.analysisDiagnostics || null,
    display: getDisplayDiagnostics(),
  })
}

/**
 * 显示浮窗
 */
const showOverlay = async (data) => {
  console.log('🔧 [FloatingOverlay] showOverlay 被调用:', data)
  const requestId = ++overlayRequestId

  visible.value = true
  loading.value = false
  error.value = null
  logFloatingInfo('showOverlay received', {
    championId: data?.championId || null,
    augmentCount: Array.isArray(data?.augments) ? data.augments.length : 0,
    augmentIds: (data?.augments || []).map(getAugmentDebugId),
    detectedSlots: (data?.augments || []).map((augment, index) => (
      Number.isInteger(augment?.detectedSlot) ? augment.detectedSlot : index
    )),
    dataSource: data?.dataSource || null,
    partialUpdate: data?.partialUpdate === true,
    winratePending: data?.winratePending === true,
    winrateInMain: data?.winrateInMain === true,
    analysisDiagnostics: data?.analysisDiagnostics || null,
  })

  if (data && data.augments && data.augments.length > 0) {
    championId.value = data.championId

    const hasRecommendationData = data.augments.some(aug => [
      aug?.rank,
      aug?.tier,
      aug?.pickRate,
      aug?.winRate,
      aug?.recommendScore,
    ].some(value => value != null && Number.isFinite(Number(value))))
    console.log('🔍 [FloatingOverlay] 检查推荐数据:', hasRecommendationData)

    if (hasRecommendationData) {
      displayAugments.value = sortAugmentsByDetectedOrder(data.augments, data.augments)
      loading.value = false
      console.log('✅ [FloatingOverlay] 直接显示完整数据')
      logDisplayState('display state applied', data, {
        mode: 'full-recommendation',
        hasRecommendationData: true,
      })
    } else {
      const fallbackAugments = mergeWinrateWithDetectedSlots([], data.augments, displayAugments.value)
      displayAugments.value = fallbackAugments
      loading.value = false
      logDisplayState('display state applied', data, {
        mode: 'fallback',
        hasRecommendationData: false,
        winratePending: data.winratePending === true,
      })
      if (data.winratePending === true || data.winrateInMain === true) {
        logFloatingInfo('fallback displayed while main winrate is pending', {
          championId: championId.value || null,
          augmentIds: fallbackAugments.map(augment => augment.id),
          display: getDisplayDiagnostics(),
          winratePending: data.winratePending === true,
          winrateInMain: data.winrateInMain === true,
          winrateError: data.winrateError || null,
        })
        return
      }

      console.log('🔍 [FloatingOverlay] 先显示识别结果，后台查询胜率数据...')
      logFloatingInfo('fallback displayed before winrate query', {
        championId: championId.value || null,
        augmentIds: fallbackAugments.map(augment => augment.id),
        display: getDisplayDiagnostics(),
      })

      try {
        // 获取当前英雄ID
        if (!championId.value) {
          // 首先尝试从store中读取缓存的英雄ID
          console.log('🔍 [FloatingOverlay] 尝试从store读取缓存的英雄ID...')
          try {
            const cachedChampionId = await electronAPI.store.get('lastSelectedChampionId')
            if (cachedChampionId) {
              championId.value = cachedChampionId
              console.log('✅ [FloatingOverlay] 从store获取到英雄ID:', cachedChampionId)
            } else {
              console.log('⚠️ [FloatingOverlay] store中没有缓存的英雄ID')
            }
          } catch (storeErr) {
            console.warn('⚠️ [FloatingOverlay] 从store读取英雄ID失败:', storeErr)
          }

          // 如果store中也没有，再尝试通过LCU API获取
          if (!championId.value) {
            console.log('🔍 [FloatingOverlay] 尝试通过LCU API获取英雄ID...')
            const championResult = await electronAPI.lcu.getChampionId()
            if (championResult.success) {
              championId.value = championResult.championId
              console.log('✅ [FloatingOverlay] 通过LCU获取到英雄ID:', championResult.championId)
            } else {
              console.warn('❌ [FloatingOverlay] LCU获取英雄ID失败:', championResult.error)
              throw new Error(t('augment.loadError'))
            }
          }
        }

        // 查询胜率数据
        const augmentIds = data.augments.map(aug => aug.id).filter(id => id != null)
        const winrateStartedAt = Date.now()
        logFloatingInfo('winrate query requested', {
          championId: championId.value,
          augmentIds,
        })

        if (augmentIds.length === 0) {
          logFloatingInfo('winrate query skipped: no recognized augment ids', {
            championId: championId.value,
          })
          return
        }

        const winrateResult = await electronAPI.winrate.get({
          championId: championId.value,
          augmentIds,
          requestStartedAt: winrateStartedAt,
          requestSource: 'augment-floating',
        })
        const winrateCompletedAt = Date.now()
        const winrateDurationMs = winrateCompletedAt - winrateStartedAt
        const winrateTiming = winrateResult?.timing || {}
        const mainToRendererDelayMs = Number.isFinite(winrateTiming.mainCompletedAt)
          ? winrateCompletedAt - winrateTiming.mainCompletedAt
          : null

        if (requestId !== overlayRequestId) {
          logFloatingInfo('winrate result ignored for stale request', {
            championId: championId.value,
            augmentIds,
            durationMs: winrateDurationMs,
            rendererToMainDelayMs: winrateTiming.rendererToMainDelayMs ?? null,
            mainDurationMs: winrateTiming.mainDurationMs ?? null,
            mainToRendererDelayMs,
          })
          return
        }

        logFloatingInfo('winrate query completed', {
          championId: championId.value,
          augmentIds,
          success: winrateResult.success,
          resultCount: winrateResult.augments?.length || 0,
          durationMs: winrateDurationMs,
          rendererToMainDelayMs: winrateTiming.rendererToMainDelayMs ?? null,
          mainDurationMs: winrateTiming.mainDurationMs ?? null,
          mainToRendererDelayMs,
        })

        if (winrateResult.success && winrateResult.augments.length > 0) {
          // 找到了胜率数据，按游戏内从左到右的识别顺序展示。
          displayAugments.value = mergeWinrateWithDetectedSlots(
            winrateResult.augments,
            data.augments,
            displayAugments.value
          )
          console.log('✅ [FloatingOverlay] 胜率数据查询成功:', winrateResult.augments)
          logDisplayState('display state applied', data, {
            mode: 'winrate-applied',
            hasWinrateData: true,
            resultCount: winrateResult.augments.length,
            durationMs: winrateDurationMs,
          })
          logFloatingInfo('winrate query applied', {
            championId: championId.value,
            augmentIds,
            resultCount: winrateResult.augments.length,
            durationMs: winrateDurationMs,
            display: getDisplayDiagnostics(),
          })
        } else if (winrateResult.success && winrateResult.augments.length === 0) {
          // 查询成功但没有这些海克斯的数据，显示基本信息
          console.warn('⚠️ [FloatingOverlay] 没有找到这些海克斯的胜率数据，显示基本信息')
        } else {
          throw new Error(winrateResult.error || t('augment.loadError'))
        }
      } catch (err) {
        console.error('❌ [FloatingOverlay] 查询失败:', err)
        logFloatingInfo('winrate query failed; keeping fallback', {
          championId: championId.value || null,
          error: err?.message || String(err),
        })
        error.value = null
      } finally {
        if (requestId === overlayRequestId) {
          loading.value = false
        }
      }
    }
  } else {
    loading.value = false
    error.value = t('augment.noData')
    logFloatingInfo('showOverlay received no data')
  }
}

/**
 * 关闭浮窗
 */
const closeOverlay = (reason = 'manual') => {
  console.log('🔧 [FloatingOverlay] 关闭浮窗')
  overlayRequestId += 1
  visible.value = false
  loading.value = false
  displayAugments.value = []
  error.value = null
  logFloatingInfo('overlay closed', { reason })

  // 隐藏浮动窗口本身
  electronAPI.windows.hideFloating(reason)
}

/**
 * 监听事件
 */
onMounted(() => {
  console.log('🔧 [FloatingOverlay] 组件已挂载')

  unsubscribeEvents.push(electronAPI.events.on('augment-detected', (data) => {
    console.log('📊 [FloatingOverlay] 收到 augment-detected:', data)
    showOverlay(data)
  }))

  unsubscribeEvents.push(electronAPI.events.on('augment-cleared', (data) => {
    console.log('🔧 [FloatingOverlay] 收到 augment-cleared:', data)
    closeOverlay('augment-cleared')
  }))

  unsubscribeEvents.push(electronAPI.events.on('game-started', () => {
    console.log('🎮 [FloatingOverlay] 游戏开始，关闭浮窗')
    closeOverlay('game-started')
  }))

  unsubscribeEvents.push(electronAPI.events.on('game-in-progress', () => {
    console.log('🎮 [FloatingOverlay] 游戏进行中，关闭浮窗')
    closeOverlay('game-in-progress')
  }))

  unsubscribeEvents.push(electronAPI.events.on('game-phase-changed', (data) => {
    if (data && (data.phase === 'GameStart' || data.phase === 'InProgress')) {
      console.log('🎮 [FloatingOverlay] 游戏阶段变化，关闭浮窗')
      closeOverlay('game-phase-changed')
    }
  }))
})

onBeforeUnmount(() => {
  unsubscribeEvents.splice(0).forEach(unsubscribe => unsubscribe())
  closeOverlay('unmount')
})
</script>

<style scoped>
.statistics-basis { margin-left: 16px; color: #c8aa6e; font-size: 11px; line-height: 1.4; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.missing-hint { color: #cdbe91; font-size: 12px; line-height: 1.6; margin: 0; }
.floating-overlay {
  position: fixed; inset: 0; box-sizing: border-box; padding: 5px 8px 8px;
  color: #f0e6d2; background: #09131ef7; border: 1px solid #665638;
  font-family: 'Segoe UI', 'Microsoft YaHei', sans-serif; overflow: hidden;
}
.floating-overlay * { box-sizing: border-box; }
.floating-header { height: 23px; display: flex; align-items: center; margin-bottom: 5px; }
.floating-title { color: #c8aa6e; font-size: 11px; font-weight: 700; letter-spacing: 2px; }
.floating-title small { margin-left: 9px; color: #a09b8c; font-size: 10px; font-weight: 400; letter-spacing: 1px; }
.close-btn { margin-left: auto; display: grid; place-items: center; width: 26px; height: 23px; padding: 3px; border: 1px solid transparent; background: transparent; color: #cdbe91; cursor: pointer; }
.close-btn:hover { background: #302a22; border-color: #785a28; }
.close-btn:focus-visible { outline: 2px solid #0ac8b9; }
.close-icon { width: 15px; height: 15px; }
.augments-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); grid-template-rows: minmax(0, 1fr); gap: 7px; height: calc(100% - 28px); }
.augment-item {
  position: relative; display: grid; grid-template-columns: 66px minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr) 30px; gap: 6px 10px; min-width: 0; padding: 10px 9px 8px;
  background: linear-gradient(125deg, #142b3c, #0a1623 75%); border: 1px solid #4c5360;
  box-shadow: inset 0 0 0 2px #07111b;
}
.augment-item::before { content: ''; position: absolute; top: -1px; left: -1px; width: 16px; height: 3px; background: #8e9caa; }
.augment-item.top-pick { border-color: #c8aa6e; background: linear-gradient(125deg, #29302f, #101d28 75%); }
.top-pick::before { background: #f0d89d; width: 34px; }
.top-pick-badge { position: absolute; right: 7px; top: -8px; padding: 0 5px; background: #17222a; color: #f0d89d; font-size: 10px; line-height: 15px; }
.top-pick-badge span { margin-right: 3px; }
.augment-icon-frame { align-self: center; width: 66px; height: 66px; padding: 3px; border: 1px solid #887044; background: #030b13; box-shadow: inset 0 0 0 2px #172e40; color: #c8aa6e; font-size: 30px; }
.augment-icon { width: 100%; height: 100%; font-size: 28px; }
.content { display: flex; flex-direction: column; justify-content: center; gap: 6px; min-width: 0; }
.name { margin: 0; color: #f0e6d2; font-size: 15px; font-weight: 650; line-height: 19px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.stats-lines { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px; }
.stat-line { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
.stat-line span { color: #a8b6bd; font-size: 10px; line-height: 14px; }
.stat-line strong { font-family: 'Bahnschrift', 'Segoe UI', sans-serif; font-size: 21px; line-height: 25px; font-weight: 600; color: #edf5f7; font-variant-numeric: tabular-nums; letter-spacing: -.6px; white-space: nowrap; }
.top-pick .stat-line strong { color: #f0d89d; }
.recommendation { grid-column: 1 / -1; min-width: 0; }
.recommend-label { display: flex; justify-content: space-between; gap: 5px; margin-bottom: 4px; color: #9badb7; font-size: 10px; line-height: 13px; }
.recommend-label strong { font-weight: 500; color: #c5d5d9; overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.score-track { height: 9px; padding: 1px; background: #020b13; border: 1px solid #365668; }
.score-fill { height: 100%; background: linear-gradient(90deg, #087888, #61d4df); }
.top-pick .score-track { border-color: #806c43; }
.top-pick .score-fill { background: linear-gradient(90deg, #956a2e, #e6c582); }
.empty-slot { border-color: #2e4050; }
.empty-slot .content { grid-row: 1 / -1; grid-column: 2; }
.empty-slot .name { font-size: 12px; color: #8998a4; }
.empty-slot .augment-icon-frame { opacity: .3; }
.loading, .error { height: calc(100% - 28px); display: flex; align-items: center; justify-content: center; gap: 12px; font-size: 12px; color: #cdbe91; }
.error { color: #efb7ad; }
.spinner { width: 18px; height: 18px; border: 2px solid #365668; border-top-color: #c8aa6e; border-radius: 50%; animation: spin .9s linear infinite; }
@keyframes spin { to { transform: rotate(360deg); } }
.float-fade-enter-active, .float-fade-leave-active { transition: opacity .12s; }
.float-fade-enter-from, .float-fade-leave-to { opacity: 0; }
@media (min-width: 950px) {
  .augment-item { grid-template-columns: 76px minmax(0, 1fr); }
  .augment-icon-frame { width: 76px; height: 76px; }
  .name { font-size: 17px; }
  .stat-line strong { font-size: 25px; line-height: 29px; }
  .stat-line span, .recommend-label { font-size: 11px; }
}
</style>
