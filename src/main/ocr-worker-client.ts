import { app, utilityProcess, type UtilityProcess } from 'electron'
import path from 'node:path'
import store from './modules/app-store.ts'
import logger from './modules/logger.ts'

let worker: UtilityProcess | null = null
let sequence = 0
let ready: Promise<void> | null = null
let settings: unknown
let gpuFailed = false
let generation = 0
let mode = 'auto'
let queue: Promise<unknown> = Promise.resolve()
let queuedTasks = 0
let idleTimer: ReturnType<typeof setTimeout> | null = null
const IDLE_RELEASE_MS = 120000
let status = { backend: 'idle', warming: false, fallback: false, reason: '' }
let pending: { id: number; resolve(value: any): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> } | null = null

export function getOcrWorkerStatus() { return { ...status } }
export function stopOcrWorker() {
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
    }
    worker = null
    ready = null
    if (pending) { clearTimeout(pending.timer); pending.reject(new Error('OCR worker exited')); pending = null }
    status = { ...status, backend: 'idle', warming: false }
  })
}
async function initialize() {
  const selected = store.get('ocr.gpuEnabled') === false ? 'cpu' : 'auto'
  if (mode !== selected) { stopOcrWorker(); gpuFailed = false; mode = selected; status.fallback = false; status.reason = '' }
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
      if (run !== generation || backend === 'cpu') { stopOcrWorker(); throw error }
      gpuFailed = true
      stopOcrWorker()
      status = { backend: 'idle', warming: true, fallback: true, reason: 'gpu-initialization-failed' }
      logger.warn('GPU OCR initialization failed; falling back to CPU')
      spawnWorker()
      const fallbackGeneration = generation
      await request({ action: 'init', backend: 'cpu', options: settings }, 10000)
      if (fallbackGeneration !== generation) throw new Error('OCR initialization cancelled', { cause: error })
      status = { ...status, backend: 'cpu', warming: false }
    }
  })()
  ready = initialization
  try { await initialization; ready = initialization } catch (error) { stopOcrWorker(); throw error }
}
function serialize<T>(task: () => Promise<T>): Promise<T> {
  if (idleTimer) clearTimeout(idleTimer)
  idleTimer = null
  queuedTasks++
  const next = queue.catch(() => undefined).then(task).finally(() => {
    queuedTasks--
    if (!queuedTasks && worker) {
      idleTimer = setTimeout(() => {
        idleTimer = null
        if (!queuedTasks && !pending) { stopOcrWorker(); logger.info('Idle OCR worker released') }
      }, IDLE_RELEASE_MS)
      idleTimer.unref()
    }
  })
  queue = next
  return next
}
export async function createOcrWorkerService(options: unknown) {
  settings = options
  await serialize(initialize)
  return {
    warmup: () => serialize(initialize),
    recognize: (input: unknown) => serialize(async () => {
      await initialize()
      const backend = status.backend
      const run = generation
      try { return await request({ action: 'recognize', input }, 6000) } catch (error) {
        if (run !== generation) throw error
        if (backend !== 'webgpu') { stopOcrWorker(); throw error }
        gpuFailed = true
        stopOcrWorker()
        status = { backend: 'idle', warming: true, fallback: true, reason: 'gpu-inference-failed' }
        logger.warn('GPU OCR inference failed; retrying current frame on CPU')
        await initialize()
        try { return await request({ action: 'recognize', input }, 6000) }
        catch (fallbackError) { stopOcrWorker(); throw fallbackError }
      }
    }),
    destroy: async () => stopOcrWorker(),
  }
}
app.once('will-quit', stopOcrWorker)
