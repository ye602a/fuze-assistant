import { readFileSync } from 'node:fs'
import vm from 'node:vm'
import { describe, expect, it, vi } from 'vitest'

function harness() {
  let callback: () => void = () => {}
  const track = { stop: vi.fn() }
  const video = { muted: false, readyState: 2, videoWidth: 4, videoHeight: 2, srcObject: null,
    requestVideoFrameCallback: (fn: () => void) => { callback = fn }, play: async () => {} }
  let pixels = new Uint8ClampedArray(32).fill(255)
  const context = { drawImage: vi.fn(), getImageData: () => ({ data: pixels.slice() }) }
  const sandbox: any = { Uint32Array, performance: { now: () => 0 },
    navigator: { mediaDevices: { getDisplayMedia: async () => ({ active: true, getTracks: () => [track] }) } },
    document: { createElement: (tag: string) => tag === 'video' ? video : { width: 0, height: 0, getContext: () => context } } }
  vm.runInNewContext(readFileSync(new URL('../../public/capture-worker.js', import.meta.url), 'utf8'), sandbox)
  return { api: sandbox.fuzeCapture, video, track, context, frame: () => callback(), setPixels: (value: Uint8ClampedArray) => { pixels = value } }
}

describe('capture pixels before IPC', () => {
  it('does not transfer a repeated frame and detects a single changed interior pixel', async () => {
    const h = harness(); await h.api.start(); h.frame()
    expect(h.api.read().pixels.byteLength).toBe(32)
    expect(h.api.read()).toMatchObject({ unchanged: true })
    expect(h.context.drawImage).toHaveBeenCalledOnce()
    h.frame()
    expect(h.api.read()).toEqual({ unchanged: true, duplicateBytes: 32 })
    const changed = new Uint8ClampedArray(32).fill(255); changed[13] = 254
    h.setPixels(changed); h.frame()
    expect(new Uint8Array(h.api.read().pixels)[13]).toBe(254)
    h.frame(); expect(h.api.read().unchanged).toBe(true)
  })
  it('sends equal bytes with changed dimensions and clears history across sessions', async () => {
    const h = harness(); await h.api.start(); h.frame(); h.api.read()
    h.video.videoWidth = 2; h.video.videoHeight = 4; h.frame()
    expect(h.api.read()).toMatchObject({ width: 2, height: 4 })
    h.api.stop(); expect(h.track.stop).toHaveBeenCalledOnce()
    await h.api.start(); h.frame()
    expect(h.api.read().pixels.byteLength).toBe(32)
  })
})
