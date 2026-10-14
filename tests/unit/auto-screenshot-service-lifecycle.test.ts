import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { recognitionMeter } from '../../src/main/recognition-summary.ts'
import { resolveCaptureStage } from '../../src/main/auto-screenshot-policy.ts'
import { createInitialPartialAugmentSelection } from '../../src/main/augment-partial-merge.ts'

const mocks = vi.hoisted(() => ({
  captureScreenshot: vi.fn(),
  analyzeScreenshot: vi.fn(),
  analyzeScreenshotGate: vi.fn(),
  warmupImageAnalyzer: vi.fn(),
  prepareWindows: vi.fn(),
  windows: [] as any[],
  logger: {
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn(),
    toBeijingISOString: vi.fn(() => '2026-08-30T12:00:00.000+08:00'),
  },
}))

vi.mock('../../src/main/live-level-reader.ts', () => ({ LiveLevelReader: class { value: { level: number; checkedAt: number } | null = null; poll = vi.fn(async () => null); reset = vi.fn(() => { this.value = null }) } }))

vi.mock('../../src/main/screenshot.ts', () => ({
  CAPTURE_THUMBNAIL_SIZE: { width: 1280, height: 720 },
  captureScreenshot: mocks.captureScreenshot,
}))

vi.mock('../../src/main/image-analyzer.ts', () => ({
  analyzeScreenshot: mocks.analyzeScreenshot,
  analyzeScreenshotGate: mocks.analyzeScreenshotGate,
  warmupImageAnalyzer: mocks.warmupImageAnalyzer,
  setOcrMatchActive: vi.fn(),
}))

vi.mock('electron', () => ({
  BrowserWindow: {
    getAllWindows: vi.fn(() => mocks.windows),
  },
}))

vi.mock('fs-extra', () => ({
  default: {
    ensureDir: vi.fn(),
    writeFile: vi.fn(),
    readdir: vi.fn(async () => []),
    stat: vi.fn(),
    remove: vi.fn(),
  },
}))

vi.mock('../../src/main/modules/logger.ts', () => ({ default: mocks.logger }))
vi.mock('../../src/main/modules/window-manager.ts', () => ({
  applyAugmentSidePanelWindowLayout: vi.fn(),
  applyFloatingWindowLayout: vi.fn(),
  raiseOverlayWindow: vi.fn(),
  ensureAugmentOverlayWindows: mocks.prepareWindows,
}))
vi.mock('../../src/main/modules/overlay-window-state.ts', () => ({
  shouldRaiseOverlayWindow: vi.fn(() => false),
}))
vi.mock('../../src/main/modules/app-store.ts', () => ({
  default: { get: vi.fn(() => null) },
}))
vi.mock('../../src/main/modules/app-paths.ts', () => ({
  getPartialOcrScreenshotDir: vi.fn(() => '/tmp/aramgg-client-ocr-test'),
}))
vi.mock('../../src/main/augment-partial-merge.ts', () => ({
  createInitialPartialAugmentSelection: vi.fn(() => null),
  getAugmentIds: vi.fn((augments = []) => augments.filter(augment => !augment.missing && augment.id != null).map(augment => String(augment.id))),
  mergePartialAugments: vi.fn(() => null),
}))
vi.mock('../../src/main/modules/user-preferences.ts', () => ({
  shouldShowAugmentSidePanel: vi.fn(() => true),
  shouldShowAugmentTopOverlay: vi.fn(() => true),
}))

import autoScreenshotService from '../../src/main/auto-screenshot-service.ts'

const createRunningIdleService = () => {
  autoScreenshotService.reset()
  autoScreenshotService.isRunning = true
  autoScreenshotService.controlOwner = 'gameflow'
  autoScreenshotService.captureMode = 'idle'
  autoScreenshotService.gameflowPhase = 'InProgress'
  autoScreenshotService.enableAnalysis = true
  autoScreenshotService.automaticThumbnailSize = { width: 1024, height: 576 }
  autoScreenshotService.pendingFullCapture = true
  autoScreenshotService.candidateStreak = 2
  return autoScreenshotService
}

const expectNextCaptureToUseGate = (service) => {
  expect(resolveCaptureStage({
    mode: service.captureMode,
    pendingFullCapture: service.pendingFullCapture,
    fullOcrCooldownUntil: service.fullOcrCooldownUntil,
    now: Date.now(),
  })).toBe('gate')
}

describe.sequential('automatic screenshot service lifecycle', () => {
  it('does not wait for optional level telemetry before capturing', async () => {
    const service = createRunningIdleService()
    service.levelReader.poll.mockReturnValue(new Promise(() => {}))
    mocks.captureScreenshot.mockResolvedValue({ success: false, error: 'fixture-no-frame' })
    await service._captureScreenshot(service.runId)
    expect(mocks.captureScreenshot).toHaveBeenCalledOnce()
  })
  it('retains a brief partial miss but replaces persistent unread slots without hiding known cards', async () => {
    vi.useFakeTimers(); vi.setSystemTime(10000)
    const service = createRunningIdleService()
    const augments = [{ id: 1, detectedSlot: 0 }, { id: 3, detectedSlot: 2 }]
    service.lastDetectedAugmentIds = ['1', '2', '3']
    service.lastDetectedAugments = [{ id: 1 }, { id: 2 }, { id: 3 }]
    service.lastDetectedAugmentAt = 10000
    const partial = [augments[0], { id: null, missing: true, detectedSlot: 1 }, augments[1]]
    vi.mocked(createInitialPartialAugmentSelection).mockReturnValue({ augments: partial, missingSlots: [1], reason: 'unreadable-title-slot' })
    mocks.analyzeScreenshot.mockResolvedValue({ success: true, analysis: {
      cardCount: 2, confidence: .95, isAugmentPhase: true, augments,
      augmentGate: { titleActivity: { likely: true }, rerollButtons: { visible: true } },
    } })
    const notify = vi.spyOn(service, '_notifyAugmentDetected').mockImplementation(() => {})
    await service._analyzeScreenshot(Buffer.from('partial'))
    expect(notify).not.toHaveBeenCalled()
    vi.setSystemTime(13000)
    await service._analyzeScreenshot(Buffer.from('partial'))
    expect(notify).toHaveBeenCalledOnce()
    expect(service.lastDetectedAugments).toEqual(partial)
    vi.mocked(createInitialPartialAugmentSelection).mockReturnValue(null)
    vi.useRealTimers()
  })
  it('releases per-game buffers, selection and timers across one hundred stop/start cycles', () => {
    vi.useFakeTimers()
    try {
      for (let i = 0; i < 100; i++) {
        const service = createRunningIdleService()
        service.lastDetectedAugmentIds = ['1', '2', '3']
        service.lastDetectedAugments = [{ id: 1 }]
        service.latestRecommendation = { augments: [{ id: 1 }] }
        service.lastRawCapture = { buffer: Buffer.alloc(64), width: 4, height: 4, channels: 4 }
        service.pendingAnalysisBuffer = Buffer.alloc(64)
        service._scheduleNextCapture(1000, service.runId)
        expect(vi.getTimerCount()).toBe(1)
        service.stop('gameflow')
        expect(vi.getTimerCount()).toBe(0)
        expect(service.lastRawCapture).toBeNull()
        expect(service.pendingAnalysisBuffer).toBeNull()
        expect(service.latestRecommendation).toBeNull()
        expect(service.lastDetectedAugments).toEqual([])
        expect(service.lastDetectedAugmentIds).toEqual([])
        expect(service.pendingFullCapture).toBe(false)
      }
    } finally { vi.useRealTimers() }
  })
  it('drops old window readiness when an identical selection reappears after clear', async () => {
    const service = createRunningIdleService()
    service.lastDetectedAugmentIds = ['1', '2', '3']
    let ready!: () => void
    mocks.prepareWindows.mockReturnValue(new Promise<void>(resolve => { ready = resolve }))
    const send = vi.fn()
    mocks.windows.push({ isDestroyed: () => false, webContents: { getURL: () => '', send } })
    const payload = { selectionRevision: service.selectionRevision, augments: [{ id: 1 }, { id: 2 }, { id: 3 }] }
    const pending = service._sendAugmentDetectedPayload(payload)
    service.clearAugmentState('stat-shard-selection')
    service.lastDetectedAugmentIds = ['1', '2', '3']
    ready(); await pending
    expect(send).not.toHaveBeenCalledWith('augment-detected', expect.anything())
    expect(service.latestRecommendation).toBeNull()
    expect(service._isCurrentAugmentPayload(payload)).toBe(false)
  })
  it('does not send an overlay during a disallowed game phase even if IDs remain', async () => {
    const service = createRunningIdleService()
    service.lastDetectedAugmentIds = ['1', '2', '3']
    service.gameflowPhase = 'EndOfGame'
    await service._sendAugmentDetectedPayload({ augments: [{ id: 1 }, { id: 2 }, { id: 3 }] })
    expect(mocks.prepareWindows).not.toHaveBeenCalled()
  })
  it('does not complete a pending offer after an OCR exception', async () => {
    recognitionMeter.begin()
    const service = createRunningIdleService(); const now = Date.now()
    service.levelGate.updateLevel(11, now)
    service.levelGate.observeSelection(true, now, 'augment')
    mocks.analyzeScreenshot.mockRejectedValueOnce(new Error('OCR failure'))
    await service._analyzeScreenshot(Buffer.from('frame'))
    expect(recognitionMeter.finish()).toMatchObject({ counts: { analyses: 1 }, lastFailure: { stage: 'analysis' } })
    for (const time of [now + 1000, now + 4000, now + 7000]) service.levelGate.observeSelection(false, time)
    expect(service.levelGate.completed.size).toBe(0)
  })
  it('coalesces repeated gameflow and level prewarming with a bounded retry interval', async () => {
    const service = createRunningIdleService()
    const now = Date.now()
    vi.spyOn(Date, 'now').mockReturnValue(now)
    await Promise.all(Array.from({ length: 30 }, () => service._warmupAnalyzer()))
    expect(mocks.warmupImageAnalyzer).toHaveBeenCalledOnce()
    await service._warmupAnalyzer()
    expect(mocks.warmupImageAnalyzer).toHaveBeenCalledOnce()
    vi.mocked(Date.now).mockReturnValue(now + 30001)
    await service._warmupAnalyzer()
    expect(mocks.warmupImageAnalyzer).toHaveBeenCalledTimes(2)
  })
  beforeEach(() => {
    vi.clearAllMocks()
    vi.mocked(createInitialPartialAugmentSelection).mockReturnValue(null)
    mocks.windows.length = 0
    mocks.prepareWindows.mockResolvedValue([])
  })

  afterEach(() => {
    autoScreenshotService.reset()
    vi.restoreAllMocks()
    vi.useRealTimers()
  })

  it('skips the actual screenshot call in level saving mode, but captures a deferred offer', async () => {
    recognitionMeter.begin()
    const service = createRunningIdleService()
    const now = Date.now()
    service.levelReader.value = { level: 4, checkedAt: now }
    service.levelGate.completed.add(3)
    service.levelGate.decide(now)
    expect(await service._captureScreenshot(service.runId)).toMatchObject({ error: 'level-gate-idle' })
    expect(mocks.captureScreenshot).not.toHaveBeenCalled()
    service.levelReader.value = { level: 8, checkedAt: now }
    mocks.captureScreenshot.mockResolvedValue({ success: false, error: 'fixture-no-frame' })
    await service._captureScreenshot(service.runId)
    expect(mocks.captureScreenshot).toHaveBeenCalledTimes(1)
    expect(recognitionMeter.finish()).toMatchObject({ counts: { levelWaits: 1, captures: 0 }, lastFailure: { stage: 'capture' } })
  })

  it('delivers the first detection only after the on-demand renderer is ready', async () => {
    const service = createRunningIdleService()
    service.lastDetectedAugmentIds = ['1', '2', '3']
    let ready!: () => void
    mocks.prepareWindows.mockReturnValue(new Promise<void>(resolve => { ready = resolve }))
    const send = vi.fn()
    mocks.windows.push({
      isDestroyed: () => false,
      webContents: { getURL: () => 'http://localhost/#/floating-overlay', send },
    })
    const payload = { augments: [{ id: 1 }, { id: 2 }, { id: 3 }] }
    const pending = service._sendAugmentDetectedPayload(payload)
    expect(send).not.toHaveBeenCalled()
    ready()
    await pending
    expect(send).toHaveBeenCalledWith('augment-detected', payload)
  })

  it('does not deliver a detection if stopped while its window is loading', async () => {
    const service = createRunningIdleService()
    service.lastDetectedAugmentIds = ['1', '2', '3']
    let ready!: () => void
    mocks.prepareWindows.mockReturnValue(new Promise<void>(resolve => { ready = resolve }))
    const send = vi.fn()
    mocks.windows.push({
      isDestroyed: () => false,
      webContents: { getURL: () => 'http://localhost/#/floating-overlay', send },
    })
    const pending = service._sendAugmentDetectedPayload({ augments: [{ id: 1 }, { id: 2 }, { id: 3 }] })
    service.stop('gameflow')
    ready()
    await pending
    expect(send).not.toHaveBeenCalledWith('augment-detected', expect.anything())
    expect(send).toHaveBeenCalledWith('augment-cleared', expect.anything())
  })

  it('does not deliver a stale detection into a restarted run with matching ids', async () => {
    const service = createRunningIdleService()
    const payload = { augments: [{ id: 1 }, { id: 2 }, { id: 3 }] }
    service.lastDetectedAugmentIds = ['1', '2', '3']
    const staleRunId = service.runId

    service.runId++
    await service._sendAugmentDetectedPayload(payload, 'detected', staleRunId)

    expect(mocks.prepareWindows).not.toHaveBeenCalled()
  })

  it('does not let a stale run occupy the restarted capture timer', () => {
    const service = createRunningIdleService()
    const staleRunId = service.runId

    service.runId++
    service._scheduleNextCapture(0, staleRunId)

    expect(service.intervalId).toBeNull()
  })

  it('ignores a gate result that finishes after the run has restarted', async () => {
    const service = createRunningIdleService()
    let resolveGate!: (value: any) => void
    mocks.analyzeScreenshotGate.mockReturnValue(new Promise(resolve => { resolveGate = resolve }))
    const staleRunId = service.runId
    const pending = service._analyzeGateScreenshot(Buffer.from('gate-frame'), staleRunId)

    service.runId++
    service.candidateStreak = 1
    service.pendingFullCapture = false
    resolveGate({ success: true, likely: true, rerollVisible: true, durationMs: 10 })
    await pending

    expect(service.candidateStreak).toBe(1)
    expect(service.pendingFullCapture).toBe(false)
  })

  it('ignores an OCR result that finishes after the run has restarted', async () => {
    const service = createRunningIdleService()
    let resolveAnalysis!: (value: any) => void
    mocks.analyzeScreenshot.mockReturnValue(new Promise(resolve => { resolveAnalysis = resolve }))
    const notify = vi.spyOn(service, '_notifyAugmentDetected').mockImplementation(() => {})
    const staleRunId = service.runId
    const pending = service._analyzeScreenshot(Buffer.from('full-frame'), staleRunId)

    service.runId++
    service.captureMode = 'idle'
    service.lastDetectedAugmentIds = []
    service.lastDetectedAugments = []
    service.detectionCount = 0
    resolveAnalysis({
      success: true,
      timestamp: Date.now(),
      analysis: {
        cardCount: 3,
        confidence: 0.95,
        isAugmentPhase: true,
        augments: [{ id: 1 }, { id: 2 }, { id: 3 }],
        slotDiagnostics: [],
        augmentGate: {
          ocrSkippedReason: null,
          titleActivity: { likely: true },
          rerollButtons: { visible: true },
        },
      },
    })
    await pending

    expect(service.captureMode).toBe('idle')
    expect(service.lastDetectedAugmentIds).toEqual([])
    expect(service.lastDetectedAugments).toEqual([])
    expect(service.detectionCount).toBe(0)
    expect(notify).not.toHaveBeenCalled()
  })

  it('consumes a queued full capture before starting OCR', async () => {
    const service = createRunningIdleService()
    const imageBuffer = Buffer.from('full-frame')
    mocks.captureScreenshot.mockResolvedValue({
      success: true,
      buffer: imageBuffer,
      width: 1024,
      height: 576,
    })
    const queueAnalysis = vi.spyOn(service, '_queueAnalysis').mockImplementation(() => {})

    const result = await service._captureScreenshot(service.runId)

    expect(result.stage).toBe('full')
    expect(service.pendingFullCapture).toBe(false)
    expect(service.candidateStreak).toBe(0)
    expect(queueAnalysis).toHaveBeenCalledWith(imageBuffer)
    expectNextCaptureToUseGate(service)
  })

  it('upgrades the same raw gate frame into OCR without a second screenshot', async () => {
    const service = createRunningIdleService()
    service.pendingFullCapture = false
    service.candidateStreak = 1
    const raw = { buffer: Buffer.alloc(16, 255), width: 2, height: 2, channels: 4 }
    mocks.captureScreenshot.mockResolvedValue({ success: true, buffer: raw })
    mocks.analyzeScreenshotGate.mockResolvedValue({ success: true, likely: true, rerollVisible: true })
    const queue = vi.spyOn(service, '_queueAnalysis').mockImplementation(() => {})
    await service._captureScreenshot(service.runId)
    expect(mocks.captureScreenshot).toHaveBeenCalledTimes(1)
    expect(mocks.analyzeScreenshotGate).toHaveBeenCalledWith(raw)
    expect(queue).toHaveBeenCalledExactlyOnceWith(raw)
    expect(service.pendingFullCapture).toBe(false)
  })

  it('confirms stationary cards using the previous raw frame, then skips duplicates', async () => {
    const service = createRunningIdleService()
    service.pendingFullCapture = false
    service.candidateStreak = 0
    const raw = { buffer: Buffer.alloc(16, 255), width: 2, height: 2, channels: 4 }
    mocks.captureScreenshot.mockResolvedValueOnce({ success: true, buffer: raw })
      .mockResolvedValue({ success: false, unchanged: true, error: 'unchanged-frame' })
    mocks.analyzeScreenshotGate.mockResolvedValue({ success: true, likely: true, rerollVisible: true })
    const queue = vi.spyOn(service, '_queueAnalysis').mockImplementation(() => {})
    await service._captureScreenshot(service.runId)
    expect(queue).not.toHaveBeenCalled()
    await service._captureScreenshot(service.runId)
    expect(queue).toHaveBeenCalledExactlyOnceWith(raw)
    await service._captureScreenshot(service.runId)
    expect(mocks.analyzeScreenshotGate).toHaveBeenCalledTimes(2)
    expect(queue).toHaveBeenCalledTimes(1)
    service.stop('gameflow')
    expect(service.lastRawCapture).toBeNull()
  })

  it('returns to gate backoff when full OCR reports failure', async () => {
    const service = createRunningIdleService()
    const startedAt = Date.now()
    mocks.analyzeScreenshot.mockResolvedValue({ success: false, error: 'ocr-failed' })

    await service._analyzeScreenshot(Buffer.from('full-frame'))

    expect(service.pendingFullCapture).toBe(false)
    expect(service.candidateStreak).toBe(0)
    expect(service.fullOcrCooldownUntil).toBeGreaterThan(startedAt)
    expectNextCaptureToUseGate(service)
  })

  it('returns a gate false positive with no confirmed cards to backoff', async () => {
    const service = createRunningIdleService()
    const startedAt = Date.now()
    mocks.analyzeScreenshot.mockResolvedValue({
      success: true,
      timestamp: Date.now(),
      analysis: {
        cardCount: 0,
        confidence: 0,
        isAugmentPhase: false,
        augments: [],
        slotDiagnostics: [],
        augmentGate: {
          ocrSkippedReason: 'selection-ui-not-confirmed',
          titleActivity: { likely: false },
          rerollButtons: { visible: false },
        },
      },
    })

    await service._analyzeScreenshot(Buffer.from('false-positive-frame'))

    expect(service.captureMode).toBe('idle')
    expect(service.pendingFullCapture).toBe(false)
    expect(service.candidateStreak).toBe(0)
    expect(service.fullOcrCooldownUntil).toBeGreaterThan(startedAt)
    expectNextCaptureToUseGate(service)
  })

  it('returns an unconfirmed partial recognition to gate backoff', async () => {
    const service = createRunningIdleService()
    const startedAt = Date.now()
    vi.spyOn(service, '_savePartialOcrScreenshot').mockImplementation(() => {})
    mocks.analyzeScreenshot.mockResolvedValue({
      success: true,
      timestamp: Date.now(),
      analysis: {
        cardCount: 1,
        confidence: 0.75,
        isAugmentPhase: false,
        augments: [{ id: 1, name: '测试海克斯', detectedSlot: 0 }],
        slotDiagnostics: [],
        augmentGate: {
          ocrSkippedReason: 'selection-ui-not-confirmed',
          titleActivity: { likely: false },
          rerollButtons: { visible: false },
        },
      },
    })

    await service._analyzeScreenshot(Buffer.from('partial-frame'))

    expect(service.captureMode).toBe('idle')
    expect(service.pendingFullCapture).toBe(false)
    expect(service.candidateStreak).toBe(0)
    expect(service.fullOcrCooldownUntil).toBeGreaterThan(startedAt)
    expectNextCaptureToUseGate(service)
  })
})
