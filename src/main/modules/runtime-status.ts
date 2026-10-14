import { app } from 'electron'
import type { RuntimeStatus } from '../../shared/runtime-status.ts'
import { getLCUServiceInstance } from '../services/lcu/lcu-service.ts'
import autoScreenshotService from '../auto-screenshot-service.ts'
import { getOcrRuntimeStatus, getOcrExecutionStatus } from '../image-analyzer.ts'
import { getCaptureStreamStatus } from '../capture-stream.ts'
import { getActiveDataStatus } from '../data-loader.ts'
import { getLatestResourceSample } from './performance-monitor.ts'
import store from './app-store.ts'
import { getClientDiscoverySnapshot } from '../services/lcu/process-auth-discovery.ts'
import { getInstalledClientDirectory } from '../services/lcu/installed-client-discovery.ts'
import { getRecognitionDataStatus } from '../services/recognition-data-service.ts'

let data: RuntimeStatus['data'] = { version: '', patch: '', state: 'loading' }
let lastDataCheck = 0
let dataPending = false

// The renderer reads cached state; it never causes LCU requests or process enumeration.
export function getRuntimeStatus(): RuntimeStatus {
  if (!dataPending && Date.now() - lastDataCheck > 60000) {
    dataPending = true
    lastDataCheck = Date.now()
    void getActiveDataStatus().then(value => {
      data = { version: value.dataVersion, patch: value.gamePatch, state: 'ready' }
    }).catch(() => { data = { ...data, state: 'error' } }).finally(() => { dataPending = false })
  }
  const config = autoScreenshotService.getConfig()
  const discovery = getClientDiscoverySnapshot()
  const installedDirectory = getInstalledClientDirectory()
  return {
    sampledAt: Date.now(), version: app.getVersion(),
    connection: getLCUServiceInstance().getConnectionStatus(),
    clientDiscovery: { ...discovery, directories: [...new Set([...discovery.directories, ...(installedDirectory ? [installedDirectory] : [])])] },
    recognitionData: getRecognitionDataStatus(),
    levelGate: autoScreenshotService.levelGateStatus,
    automatic: store.get('autoScreenshotGameflowControl') !== false,
    capture: {
      error: config.lastCaptureError || null,
      running: config.isRunning, capturing: config.isCapturing, analyzing: config.isAnalyzing,
      count: config.screenshotCount, analysisCount: config.analysisCount,
      lastCaptureAt: config.lastScreenshotTime || null,
      lastAnalysisMs: config.lastAnalysisDuration,
    },
    ocr: getOcrRuntimeStatus(), data: { ...data },
    ocrExecution: getOcrExecutionStatus(), captureExecution: getCaptureStreamStatus(),
    recent: autoScreenshotService.getRecentAugmentDiagnostic(),
    resources: getLatestResourceSample(),
  }
}
