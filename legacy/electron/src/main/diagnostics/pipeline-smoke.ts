import { app, nativeImage } from 'electron'
import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { bitmapToRgba } from '../../shared/raw-frame.ts'

/** Only the explicitly opted-in, temporary packaged release harness may invoke this. */
export async function runPipelineSmoke(output: string) {
  assert.equal(process.env.ARAMGG_RELEASE_SMOKE_TEST, '1')
  assert.equal(app.isPackaged, true)
  const { getAppDataDir } = await import('../modules/app-paths.ts')
  const dataRoot = getAppDataDir()
  const relative = path.relative(os.tmpdir(), dataRoot)
  assert(!relative.startsWith('..') && !path.isAbsolute(relative), 'Diagnostic data must be temporary')
  const fixtures = process.env.FUZE_PIPELINE_FIXTURES!
  assert(path.isAbsolute(fixtures), 'Explicit fixtures directory required')
  const manifest = JSON.parse(await fs.readFile(path.join(fixtures, 'manifest.json'), 'utf8'))
  const augments = [...new Map<number, any>(manifest.flatMap((sample: any) => sample.expectedIds.map((id: number, i: number) => [id, { id, name: sample.expectedNames[i], rarity: 'kGold', iconPath: '' }]))).values()]
  const dataVersion = '99.99.99'
  for (const locale of ['zh-CN', 'en-US', 'zh-TW']) {
    const root = path.join(dataRoot, 'data')
    const versionDir = locale === 'zh-CN' ? path.join(root, 'versions', dataVersion) : path.join(root, 'versions', locale, dataVersion)
    await fs.mkdir(versionDir, { recursive: true })
    await fs.writeFile(path.join(versionDir, 'augments.json'), JSON.stringify({ augments }))
    await fs.writeFile(path.join(versionDir, 'manifest.json'), JSON.stringify({ locale, dataVersion, files: [{ path: 'augments.json' }] }))
    await fs.writeFile(path.join(root, locale === 'zh-CN' ? 'current.json' : `current.${locale}.json`), JSON.stringify({ schemaVersion: 3, locale, dataVersion }))
  }
  process.env.ARAMGG_OCR_LOCALE = 'zh-CN'
  const analyzer = await import('../image-analyzer.ts')
  const { default: store } = await import('../modules/app-store.ts')
  const { getOcrWorkerStatus, stopOcrWorker } = await import('../ocr-worker-client.ts')
  const capture = await import('../capture-stream.ts')
  const observations: any[] = []
  try {
    for (const gpu of [true, false]) {
      store.set('ocr.gpuEnabled', gpu)
      let heartbeats = 0
      const ticker = setInterval(() => heartbeats++, 20)
      const start = performance.now()
      try { assert.equal(await analyzer.warmupImageAnalyzer(), true) } finally { clearInterval(ticker) }
      const state = getOcrWorkerStatus()
      assert.equal(state.backend, gpu ? 'webgpu' : 'cpu')
      assert(heartbeats > 0, 'OCR initialization must not block main process')
      observations.push({ mode: state.backend, warmupMs: performance.now() - start, heartbeats })
      for (const sample of manifest) {
        assert.equal(path.basename(sample.file), sample.file)
        const png = await fs.readFile(path.join(fixtures, sample.file))
        const image = nativeImage.createFromBuffer(png).resize({ width: 1024, height: 576 })
        const raw = bitmapToRgba(image.toBitmap(), 1024, 576)
        for (const [kind, input] of [['raw', raw], ['png', image.toPNG()]] as const) {
          const result = await analyzer.analyzeScreenshot(input)
          assert.equal(result.success, true)
          assert('analysis' in result && 'metadata' in result)
          assert.deepEqual(result.analysis.augments.map((item: any) => Number(item.id)), sample.expectedIds)
          assert.equal(result.analysis.cardCount, sample.expectedCardCount)
          observations.push({ mode: state.backend, file: sample.file, kind, count: result.analysis.cardCount, ms: result.metadata.analysisDurationMs })
        }
      }
      if (gpu) {
        // Terminate only this temporary test app's named OCR child; never touch another app.
        const child = app.getAppMetrics().find(metric => metric.name === 'Fuze OCR' || metric.serviceName === 'Fuze OCR')
        assert(child && child.pid !== process.pid, 'Diagnostic OCR child must be identifiable')
        process.kill(child.pid)
        for (let i = 0; i < 30 && getOcrWorkerStatus().backend !== 'idle'; i++) await new Promise(resolve => setTimeout(resolve, 50))
        assert.equal(getOcrWorkerStatus().backend, 'idle')
        const start = performance.now()
        const sample = manifest[0]
        const result = await analyzer.analyzeScreenshot(await fs.readFile(path.join(fixtures, sample.file)))
        assert(result.success && 'analysis' in result)
        assert.deepEqual(result.analysis.augments.map((item: any) => Number(item.id)), sample.expectedIds)
        assert.equal(getOcrWorkerStatus().backend, 'cpu')
        assert.equal(getOcrWorkerStatus().fallback, true)
        observations.push({ recovery: 'gpu-child-exit-to-cpu', ms: performance.now() - start, cardCount: result.analysis.cardCount })
      }
    }
    // Capture only in memory; never save, return or OCR any desktop content.
    let frame = null
    const started = performance.now()
    for (let i = 0; i < 30; i++) {
      const candidate = await capture.readCaptureStream()
      if (candidate && candidate !== 'unchanged') { frame = candidate; break }
      if (capture.getCaptureStreamStatus().backend === 'bitmap-fallback') throw new Error('Persistent capture failed: ' + capture.getCaptureStreamStatus().error)
      await new Promise(resolve => setTimeout(resolve, 100))
    }
    assert(frame, 'Capture stream must produce pixels')
    observations.push({ captureStartMs: performance.now() - started, width: frame.width, height: frame.height })
    for (let i = 0; i < 5; i++) {
      await new Promise(resolve => setTimeout(resolve, 550))
      const start = performance.now()
      await capture.readCaptureStream()
      observations.push({ captureReadMs: performance.now() - start })
    }
    assert.equal(capture.getCaptureStreamStatus().backend, 'stream')
    observations.push({ captureState: capture.getCaptureStreamStatus() })
    await fs.writeFile(path.join(output, 'pipeline-report.json'), JSON.stringify({ passed: true, observations }, null, 2))
  } finally {
    capture.stopCaptureStream()
    stopOcrWorker()
    await analyzer.shutdownImageAnalyzer()
  }
}
