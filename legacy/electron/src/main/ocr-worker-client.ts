import { app, utilityProcess, type UtilityProcess } from 'electron'
import path from 'node:path'
import { recognitionMeter } from './recognition-summary.ts'
import store from './modules/app-store.ts'
import logger from './modules/logger.ts'

let worker: UtilityProcess | null = null
let sequence = 0
let ready: Promise<void> | null = null
let settings: unknown
let gpuFailed = false
let generation = 0
let lifecycle = 0
let quitting = false
let mode = 'auto'
let queue: Promise<unknown> = Promise.resolve()
let queuedTasks = 0
let idleTimer: ReturnType<typeof setTimeout> | null = null
const IDLE_RELEASE_MS = 120000
let matchActive = false
function scheduleIdleRelease() {
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = null
  if (matchActive || queuedTasks || !worker) return
  idleTimer = setTimeout(() => {
    idleTimer = null
    if (!matchActive && !queuedTasks && !pending) { releaseWorker(); logger.info('Idle OCR worker released') }
  }, IDLE_RELEASE_MS)
  idleTimer.unref()
}
// Keep a single initialized model during a match; this performs no inference or polling.
export function setOcrMatchActive(active: boolean) { matchActive = active; scheduleIdleRelease() }
let status = { backend: 'idle', warming: false, fallback: false, reason: '' }
let pending: { id: number; resolve(value: any): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> } | null = null

export function getOcrWorkerStatus() { return { ...status } }
export function stopOcrWorker() {
  matchActive = false
  lifecycle++
  releaseWorker()
}
// Backend recovery and idle eviction keep the service usable; explicit stop cancels its queue.
function releaseWorker() {
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = null
  generation++
  ready = null
  const previous = worker
  worker = null
  if (pending) { clearTimeout(pending.timer); pending.reject(new Error('OCR worker stopped')); pending = null }
  previous?.kill()
  status = { ...status, backend: 'idle', warming: false }
}
function request(payload: Record<string, unknown>, timeout: number): Promise<any> {
  if (!worker || pending) return Promise.reject(new Error('OCR worker unavailable or busy'))
  const target = worker
  return new Promise((resolve, reject) => {
    const id = ++sequence
    const timer = setTimeout(() => { if (pending?.id === id) { pending = null; reject(new Error('OCR worker timeout')) } }, timeout)
    pending = { id, resolve, reject, timer }
    try { target.postMessage({ ...payload, id }) } catch (error) { clearTimeout(timer); pending = null; reject(error) }
  })
}
function spawnWorker() {
  const child = utilityProcess.fork(path.join(app.getAppPath(), 'dist-electron', 'ocr-worker.js'), [], { serviceName: 'Fuze OCR', stdio: 'ignore' })
  worker = child
  child.on('message', (message) => {
    if (worker !== child || !pending || message.id !== pending.id) return
    const waiter = pending
    pending = null
    clearTimeout(waiter.timer)
    if (message.error) waiter.reject(new Error(String(message.error).slice(0, 200)))
    else waiter.resolve(message.result)
  })
  child.on('exit', () => {
    if (worker !== child) return
    if (status.backend === 'webgpu') {
      gpuFailed = true
      status = { ...status, fallback: true, reason: 'gpu-worker-exited' }
      recognitionMeter.fallback('gpu-worker-exited', recognitionMeter.ticket())
    }
    worker = null
    ready = null
    if (pending) { clearTimeout(pending.timer); pending.reject(new Error('OCR worker exited')); pending = null }
    status = { ...status, backend: 'idle', warming: false }
  })
}
async function initialize() {
  const metricTicket = recognitionMeter.ticket()
  const selected = store.get('ocr.gpuEnabled') === false ? 'cpu' : 'auto'
  if (mode !== selected) { releaseWorker(); gpuFailed = false; mode = selected; status.fallback = false; status.reason = '' }
  if (ready) return ready
  const run = generation
  const initialization = (async () => {
    const backend = selected === 'auto' && !gpuFailed && process.platform === 'win32' ? 'webgpu' : 'cpu'
    status = { ...status, backend: 'idle', warming: true }
    spawnWorker()
    try {
      await request({ action: 'init', backend, options: settings }, 15000)
      if (run !== generation) throw new Error('OCR initialization cancelled')
      status = { ...status, backend, warming: false }
      logger.info('OCR worker warmed', { backend, fallback: gpuFailed })
    } catch (error) {
      if (run !== generation || backend === 'cpu') { releaseWorker(); throw error }
      gpuFailed = true
      releaseWorker()
      status = { backend: 'idle', warming: true, fallback: true, reason: 'gpu-initialization-failed' }
      recognitionMeter.fallback('gpu-initialization-failed', metricTicket)
      logger.warn('GPU OCR initialization failed; falling back to CPU')
      spawnWorker()
      const fallbackGeneration = generation
      await request({ action: 'init', backend: 'cpu', options: settings }, 10000)
      if (fallbackGeneration !== generation) throw new Error('OCR initialization cancelled', { cause: error })
      status = { ...status, backend: 'cpu', warming: false }
    }
  })()
  ready = initialization
  try { await initialization; ready = initialization } catch (error) { releaseWorker(); throw error }
}
function serialize<T>(task: (assertActive: () => void) => Promise<T>): Promise<T> {
  const run = lifecycle
  const assertActive = () => {
    if (quitting || run !== lifecycle) throw new Error('OCR worker stopped')
  }
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = null
  queuedTasks++
  const next = queue.catch(() => undefined).then(() => { assertActive(); return task(assertActive) }).finally(() => {
    queuedTasks--
    scheduleIdleRelease()
  })
  queue = next
  return next
}
export async function createOcrWorkerService(options: unknown) {
  settings = options
  await serialize(initialize)
  let destroyed = false
  function submit<T>(task: (assertActive: () => void) => Promise<T>): Promise<T> {
    if (destroyed) return Promise.reject(new Error('OCR worker stopped'))
    return serialize(task)
  }
  return {
    warmup: () => submit(initialize),
    recognize: (input: unknown) => {
      const metricTicket = recognitionMeter.ticket()
      return submit(async assertActive => {
        await initialize()
        assertActive()
        const backend = status.backend
        const run = generation
        recognitionMeter.inference(backend, metricTicket)
        try { return await request({ action: 'recognize', input }, 6000) } catch (error) {
          if (run !== generation) throw error
          if (backend !== 'webgpu') { recognitionMeter.fail('ocr', metricTicket); releaseWorker(); throw error }
          gpuFailed = true
          releaseWorker()
          status = { backend: 'idle', warming: true, fallback: true, reason: 'gpu-inference-failed' }
          recognitionMeter.fallback('gpu-inference-failed', metricTicket)
          logger.warn('GPU OCR inference failed; retrying current frame on CPU')
          await initialize()
          assertActive()
          recognitionMeter.inference('cpu', metricTicket)
          try { return await request({ action: 'recognize', input }, 6000) }
          catch (fallbackError) { recognitionMeter.fail('ocr', metricTicket); releaseWorker(); throw fallbackError }
        }
      })
    },
    destroy: async () => { if (!destroyed) { destroyed = true; stopOcrWorker() } },
  }
}
app.once('will-quit', () => { quitting = true; stopOcrWorker() })
