import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const state = vi.hoisted(() => ({ windows: [] as any[], enumerate: vi.fn(), pixels: () => new Uint8Array(16).fill(255).buffer, fail: false, hang: false }))
vi.mock('electron', () => ({
  app: { getAppPath: () => '/app', isPackaged: true, once: vi.fn() },
  screen: { getPrimaryDisplay: () => ({ id: 1 }) },
  desktopCapturer: { getSources: state.enumerate },
  BrowserWindow: class {
    destroyed = false
    webContents = {
      setWindowOpenHandler: vi.fn(), on: vi.fn(), mainFrame: {},
      session: { setPermissionRequestHandler: vi.fn(), setPermissionCheckHandler: vi.fn(), setDisplayMediaRequestHandler: vi.fn() },
      executeJavaScript: vi.fn(async (code: string) => {
        if (state.fail) throw new Error('capture failed')
        if (state.hang) return new Promise(() => {})
        return code.includes('start') ? true : { width: 2, height: 2, pixels: state.pixels() }
      }),
    }
    constructor() { state.windows.push(this) }
    loadFile = vi.fn(async () => {})
    isDestroyed() { return this.destroyed }
    destroy() { this.destroyed = true }
  },
}))
vi.mock('../../src/main/modules/logger.ts', () => ({ default: { info: vi.fn(), warn: vi.fn() } }))
beforeEach(() => {
  vi.resetModules(); state.windows = []; state.fail = false; state.hang = false
  state.enumerate.mockReset().mockResolvedValue([{ id: 'screen:1', display_id: '1' }])
})
afterEach(() => vi.useRealTimers())
describe('persistent capture lifecycle', () => {
  it('enumerates once, deduplicates pixels and releases the hidden capture window', async () => {
    const capture = await import('../../src/main/capture-stream.ts')
    expect(await capture.readCaptureStream()).toMatchObject({ width: 2, height: 2 })
    expect(await capture.readCaptureStream()).toBe('unchanged')
    expect(state.enumerate).toHaveBeenCalledTimes(1)
    expect(capture.getCaptureStreamStatus()).toMatchObject({ frames: 1, duplicates: 1 })
    capture.stopCaptureStream()
    expect(state.windows[0].destroyed).toBe(true)
    expect(capture.getCaptureStreamStatus().backend).toBe('idle')
  })
  it('backs off a failed capture without opening repeated windows', async () => {
    state.fail = true
    const capture = await import('../../src/main/capture-stream.ts')
    expect(await capture.readCaptureStream()).toBeNull()
    expect(await capture.readCaptureStream()).toBeNull()
    expect(state.enumerate).toHaveBeenCalledTimes(1)
    expect(state.windows[0].destroyed).toBe(true)
    expect(capture.getCaptureStreamStatus().backend).toBe('bitmap-fallback')
  })
  it('destroys a stalled capture session at the deadline', async () => {
    vi.useFakeTimers(); state.hang = true
    const capture = await import('../../src/main/capture-stream.ts')
    const reading = capture.readCaptureStream()
    await vi.advanceTimersByTimeAsync(8001)
    expect(await reading).toBeNull()
    expect(state.windows[0].destroyed).toBe(true)
  })
  it('does not resurrect a capture stopped during source discovery', async () => {
    let resolve!: (sources: unknown[]) => void
    state.enumerate.mockReturnValue(new Promise(done => { resolve = done }))
    const capture = await import('../../src/main/capture-stream.ts')
    const reading = capture.readCaptureStream()
    capture.stopCaptureStream()
    resolve([{ id: 'screen:1', display_id: '1' }])
    expect(await reading).toBe('unchanged')
    expect(state.windows).toHaveLength(0)
  })
})
