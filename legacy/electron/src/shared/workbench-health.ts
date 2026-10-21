import { connectionIsVerified, runtimeState, type RuntimeStatus } from './runtime-status.ts'

export type HealthTone = 'blue' | 'green' | 'yellow' | 'red'
export interface ModuleHealth { key: string; icon: string; tone: HealthTone; label: string; hint: string }
export function workbenchHealth(status: RuntimeStatus | null, unavailable = false) {
  const module = (key: string, icon: string, tone: HealthTone, label: string, hint: string): ModuleHealth => ({ key, icon, tone, label, hint })
  if (!status || unavailable) return { tone: (unavailable ? 'red' : 'blue') as HealthTone,
    modules: ['client', 'capture', 'ocr', 'data'].map((key, i) => module(key, ['network', 'capture', 'recognition', 'data'][i], unavailable ? 'red' : 'blue', unavailable ? 'unavailable' : 'checking', 'healthRefresh')) }
  const connected = connectionIsVerified(status.connection, status.sampledAt)
  const playing = connected && status.connection.phase === 'InProgress'
  const active = playing && status.automatic
  const state = runtimeState(status, status.sampledAt)
  const phase = status.connection.phase
  const phaseHint = phase === 'ChampSelect' ? 'champSelect' : phase === 'GameStart' ? 'loadingGame' : phase === 'InProgress' ? 'healthInGame' : 'lobby'
  const connection = module('client', 'network', connected ? 'green' : status.connection.discovered ? 'red' : 'blue', connected ? 'connected' : status.connection.discovered ? 'connectionError' : 'waiting', connected ? phaseHint : 'healthAutoConnect')
  const capture = !active ? module('capture', 'capture', status.automatic ? 'blue' : 'yellow', status.automatic ? 'healthStandby' : 'paused', 'healthCaptureStandby')
    : status.capture.error ? module('capture', 'capture', 'red', 'captureError', 'healthCaptureError')
    : status.levelGate?.mode === 'saving' ? module('capture', 'capture', 'green', 'healthSaving', 'healthSavingHint')
    : !status.capture.running || !status.capture.lastCaptureAt ? module('capture', 'capture', 'yellow', 'healthPreparing', 'healthCapturePreparing')
    : module('capture', 'capture', status.captureExecution?.backend === 'bitmap-fallback' ? 'yellow' : 'green', 'healthCapturing', status.captureExecution?.backend === 'bitmap-fallback' ? 'healthCaptureFallback' : 'healthCaptureWorking')
  const ocr = !active && status.ocr === 'idle' && !status.ocrExecution?.warming ? module('ocr', 'recognition', 'blue', 'healthStandby', 'healthOcrStandby')
    : status.ocr === 'error' ? module('ocr', 'recognition', 'red', 'ocrError', 'healthOcrError')
    : status.ocrExecution?.warming || status.ocr === 'loading' ? module('ocr', 'recognition', 'yellow', 'warming', 'healthWarmup')
    : status.ocr !== 'ready' ? module('ocr', 'recognition', 'yellow', 'healthPreparing', 'healthWarmup')
    : module('ocr', 'recognition', status.ocrExecution?.fallback ? 'yellow' : 'green', status.capture.analyzing ? 'healthRecognizing' : 'healthReady', status.ocrExecution?.fallback ? 'healthCpuFallback' : status.ocrExecution?.backend === 'webgpu' ? 'healthGpuReady' : 'healthCpuReady')
  const data = module('data', 'data', status.data.state === 'error' ? 'red' : status.data.state === 'ready' ? 'green' : 'yellow', status.data.state === 'ready' ? 'healthDataReady' : status.data.state === 'error' ? 'healthDataError' : 'loading', status.data.state === 'ready' ? 'healthDataHint' : 'healthDataWaiting')
  const modules = [connection, capture, ocr, data]
  const tone: HealthTone = modules.some(m => m.tone === 'red') ? 'red' : !status.automatic || (playing && modules.some(m => m.tone === 'yellow')) ? 'yellow' : ['inGame', 'savingGame'].includes(state) ? 'green' : 'blue'
  return { tone, modules }
}
