import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'

const harness = vi.hoisted(() => ({ gpu: true, failure: '', children: [] as any[] }))
vi.mock('electron', () => ({ app: { once: vi.fn(), getAppPath: () => '/app' }, utilityProcess: { fork: () => {
  const child = new EventEmitter() as any
  child.backend = ''
  child.kill = vi.fn()
  child.postMessage = (message: any) => {
    if (message.action === 'init') child.backend = message.backend
    if (child.backend === 'webgpu' && harness.failure === 'timeout' && message.action === 'recognize') return
    queueMicrotask(() => {
      if (child.backend === 'webgpu' && harness.failure === 'exit' && message.action === 'recognize') { child.emit('exit', 1); return }
      const fail = child.backend === 'webgpu' && harness.failure === message.action
      child.emit('message', fail ? { id: message.id, error: 'test GPU failure' } : { id: message.id, result: message.action === 'init' ? { warmed: true } : ['text'] })
    })
  }
  harness.children.push(child)
  return child
} } }))
vi.mock('../../src/main/modules/app-store.ts', () => ({ default: { get: () => harness.gpu } }))
vi.mock('../../src/main/modules/logger.ts', () => ({ default: { info: vi.fn(), warn: vi.fn() } }))

beforeEach(() => { vi.resetModules(); harness.gpu = true; harness.failure = ''; harness.children = [] })
afterEach(() => vi.useRealTimers())
describe('isolated OCR recovery', () => {
  it('does not accept new warmup after the app has begun quitting', async () => {
    const client = await import('../../src/main/ocr-worker-client.ts')
    const { app } = await import('electron')
    const service = await client.createOcrWorkerService({})
    const callback = vi.mocked(app.once).mock.calls.findLast(call => call[0] === 'will-quit')![1]
    callback({} as never)
    await expect(service.warmup()).rejects.toThrow('stopped')
    await expect(client.createOcrWorkerService({})).rejects.toThrow('stopped')
    expect(harness.children).toHaveLength(1)
    expect(client.getOcrWorkerStatus().backend).toBe('idle')
  })
  it('releases every worker and idle timer over one hundred service lifetimes', async () => {
    vi.useFakeTimers()
    const client = await import('../../src/main/ocr-worker-client.ts')
    for (let i = 0; i < 100; i++) {
      const service = await client.createOcrWorkerService({})
      await service.recognize({})
      await service.destroy()
      expect(vi.getTimerCount()).toBe(0)
      expect(client.getOcrWorkerStatus().backend).toBe('idle')
    }
    expect(harness.children).toHaveLength(100)
    expect(harness.children.every(child => child.kill.mock.calls.length === 1)).toBe(true)
  })
  it('releases idle models after two minutes and reinitializes on the next request', async () => {
    vi.useFakeTimers()
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    await vi.advanceTimersByTimeAsync(119000)
    expect(harness.children[0].kill).not.toHaveBeenCalled()
    await service.recognize({})
    await vi.advanceTimersByTimeAsync(119000)
    expect(client.getOcrWorkerStatus().backend).toBe('webgpu')
    await vi.advanceTimersByTimeAsync(1001)
    expect(harness.children[0].kill).toHaveBeenCalledOnce()
    expect(client.getOcrWorkerStatus().backend).toBe('idle')
    expect(await service.recognize({})).toEqual(['text'])
    expect(harness.children).toHaveLength(2)
    await service.destroy()
  })
  it('prewarms GPU once and keeps serialized requests on the same process', async () => {
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    expect(client.getOcrWorkerStatus().backend).toBe('webgpu')
    await Promise.all([service.recognize({}), service.recognize({}), service.warmup()])
    expect(harness.children).toHaveLength(1)
    await service.destroy()
  })
  it.each(['init', 'recognize', 'exit'])('falls back and latches CPU after GPU %s failure', async failure => {
    harness.failure = failure
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    expect(await service.recognize({})).toEqual(['text'])
    expect(client.getOcrWorkerStatus()).toMatchObject({ backend: 'cpu', fallback: true })
    await service.recognize({})
    expect(harness.children).toHaveLength(2)
    await service.destroy()
  })
  it('kills a stalled GPU process and retries that frame on CPU', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    harness.failure = 'timeout'
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    const result = service.recognize({})
    await vi.advanceTimersByTimeAsync(6001)
    expect(await result).toEqual(['text'])
    expect(harness.children[0].kill).toHaveBeenCalledOnce()
    expect(client.getOcrWorkerStatus().backend).toBe('cpu')
    await service.destroy()
  })
  it('honors explicit CPU selection without touching GPU', async () => {
    harness.gpu = false
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    expect(harness.children[0].backend).toBe('cpu')
    await service.destroy()
  })
  it('does not resurrect a worker stopped during inference', async () => {
    harness.failure = 'timeout'
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    const result = service.recognize({})
    const rejected = expect(result).rejects.toThrow('stopped')
    await new Promise(resolve => setTimeout(resolve, 10))
    await service.destroy()
    await rejected
    expect(harness.children).toHaveLength(1)
    expect(client.getOcrWorkerStatus().backend).toBe('idle')
  })
  it('cancels queued frames and warmup on destroy, and permits a fresh service', async () => {
    harness.failure = 'timeout'
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    const results = Promise.allSettled([service.recognize({}), service.recognize({}), service.warmup()])
    await new Promise(resolve => setTimeout(resolve, 10))
    await service.destroy()
    harness.failure = ''
    expect((await results).map(result => result.status)).toEqual(['rejected', 'rejected', 'rejected'])
    expect(harness.children).toHaveLength(1)
    await expect(service.recognize({})).rejects.toThrow('stopped')
    const fresh = await client.createOcrWorkerService({})
    expect(await fresh.recognize({})).toEqual(['text'])
    await fresh.destroy()
  })
  it('keeps queued frames usable through an internal GPU fallback', async () => {
    harness.failure = 'recognize'
    const client = await import('../../src/main/ocr-worker-client.ts')
    const service = await client.createOcrWorkerService({})
    expect(await Promise.all([service.recognize({}), service.recognize({}), service.warmup()]))
      .toEqual([['text'], ['text'], undefined])
    expect(harness.children).toHaveLength(2)
    await service.destroy()
  })
})
