<template>
  <transition name="float-fade">
    <div v-if="visible" class="floating-overlay">
      <div class="floating-header"><span class="floating-title">FUZE <small>{{ t('augment.recommendation') }}</small></span><span class="order-hint">01 / 02 / 03</span><button class="close-btn" type="button" @click="closeOverlay('manual')" :title="t('common.close')" :aria-label="t('common.close')">
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

          <div class="slot-number">{{ String(index + 1).padStart(2, '0') }}</div><div class="augment-icon-frame">
            <GameIcon
              v-if="!augment.missing && augment.iconPath"
              :src="getAugmentIconUrl(augment.iconPath)"
              :alt="augment.name"
              class="augment-icon"
            />
            <span v-else-if="!augment.missing">{{ String(index + 1).padStart(2, '0') }}</span>
          </div>

          <div class="content">
            <h3 class="name">{{ augment.missing ? t('workbench.missing') : augment.name }}</h3>
            <div v-if="!augment.missing" class="stats-lines">
              <div class="stat-line">
                <span>{{ t('augment.pickRateShort') }}</span>
                <strong>{{ formatPercent(augment.pickRate) }}</strong>
              </div>
              <div class="stat-line">
                <span>{{ t('augment.winRate') }}</span>
                <strong>{{ formatAugmentWinRate(augment.winRate) }}</strong>
              </div>
            </div>
            <span v-if="!augment.missing" class="recommend-label" :class="getBadgeClass(augment.recommendScore)">
              <template v-if="augment.statistics">{{ t('workbench.stats_' + augment.statistics.status) }} {{ augment.statistics.patch || '' }}</template>
              <template v-else>{{ getRecommendText(augment.recommendScore) }} · {{ formatScore(augment.recommendScore) }}</template>
            </span>
          </div>

          <div v-if="!augment.missing" class="score-track">
            <div class="score-fill" :style="{ width: getScoreWidth(augment.recommendScore) }"></div>
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

/**
 * 获取徽章样式
 */
const getBadgeClass = (score) => {
  if (score == null || isNaN(score)) return 'unknown'
  if (score >= 0.6) return 'must'
  if (score >= 0.5) return 'strong'
  if (score >= 0.4) return 'good'
  if (score >= 0.3) return 'ok'
  return 'cold'
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

.floating-overlay{position:fixed;inset:0;width:100%;height:100%;box-sizing:border-box;padding:8px 10px 10px;background:#101c2af5;border:1px solid #395064;border-radius:12px;color:#e5edf8;font-family:'Segoe UI','Microsoft YaHei',sans-serif;overflow:hidden}
.floating-overlay *{box-sizing:border-box}.floating-header{height:24px;display:flex;align-items:center;gap:12px;margin-bottom:7px}.floating-title{font-size:11px;font-weight:800;letter-spacing:1.8px;color:#87e5d2}.floating-title small{font-size:10px;font-weight:400;letter-spacing:0;color:#a9bace;margin-left:9px}.order-hint{margin-left:auto;font-size:10px;letter-spacing:2px;color:#7790aa}.close-btn{width:25px;height:23px;padding:4px;border:0;border-radius:5px;color:#b7c8de;background:#27374c;cursor:pointer}.close-btn:hover{background:#59303e}.close-icon{width:15px;height:15px}.close-btn:focus-visible{outline:2px solid #7bdcc8}
.augments-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:8px;height:calc(100% - 31px)}.augment-item{position:relative;display:grid;grid-template-columns:38px minmax(0,1fr);align-items:start;gap:9px;padding:23px 10px 12px;border:1px solid #304458;border-radius:8px;background:#1a293a;min-width:0}.augment-item.top-pick{border-color:#78ddc7;background:#193632}.slot-number{position:absolute;top:5px;left:10px;font-size:10px;letter-spacing:1.6px;color:#819cb9}.top-pick-badge{position:absolute;right:8px;top:4px;display:flex;gap:4px;color:#91f1d9;font-size:10px}.augment-icon-frame{height:38px;width:38px;display:grid;place-items:center;background:#26394c;border:1px solid #496479;border-radius:9px;color:#87a5c4;font-size:12px;overflow:hidden}.augment-icon{width:100%;height:100%;object-fit:cover}.content{display:flex;flex-direction:column;gap:5px;min-width:0}.name{margin:0;font-size:15px;font-weight:650;line-height:20px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;color:#e7eff8}.stats-lines{display:flex;gap:12px}.stat-line{display:flex;flex-direction:column;gap:2px}.stat-line span{font-size:10px;color:#9eb2ca}.stat-line strong{font-size:13px;color:#d7e5f5;font-variant-numeric:tabular-nums}.top-pick .stat-line strong{color:#9aefda}.recommend-label{font-size:9px;color:#a4bbcd;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.score-track{position:absolute;bottom:5px;left:10px;right:10px;height:2px;background:#314356;border-radius:3px;overflow:hidden}.score-fill{height:100%;background:#73d9c2}.empty-slot{border-style:dashed;background:#152334}.empty-slot .name{color:#7d96ae;font-size:12px;font-weight:400}.loading,.error{height:100%;display:flex;justify-content:center;align-items:center;gap:12px;font-size:12px;color:#a4bbcd}.error{color:#f3acaf}.spinner{width:18px;height:18px;border:2px solid #355168;border-top-color:#7adeca;border-radius:50%;animation:spin .9s linear infinite}@keyframes spin{to{transform:rotate(360deg)}}.float-fade-enter-active,.float-fade-leave-active{transition:opacity .12s}.float-fade-enter-from,.float-fade-leave-to{opacity:0}
@media(min-width:950px){.augment-item{grid-template-columns:44px minmax(0,1fr);padding-top:25px}.augment-icon-frame{width:44px;height:44px}.name{font-size:17px}.stat-line strong{font-size:14px}.recommend-label{font-size:10px}}
</style>
