import { readFileSync } from 'node:fs'
import { afterEach, describe, expect, it, vi } from 'vitest'
const payload = JSON.parse(readFileSync(new URL('../../resources/recognition-catalog.json', import.meta.url), 'utf8'))

describe('independent cached provider updates', () => {
  afterEach(() => { vi.useRealTimers(); vi.unstubAllGlobals(); vi.resetModules(); vi.doUnmock('node:fs/promises'); vi.doUnmock('../../src/main/modules/app-paths.ts') })
  it('uses bundled data offline and commits only validated responses without rollback', async () => {
    const files = new Map<string, string>()
    const write = vi.fn(async (name: string, value: string) => { files.set(name, value) })
    vi.doMock('../../src/main/modules/app-paths.ts', () => ({ getAppDataDir: () => 'TEST-ONLY' }))
    vi.doMock('node:fs/promises', () => ({
      mkdir: vi.fn(), writeFile: write,
      rename: vi.fn(async (a: string, b: string) => { files.set(b, files.get(a)!); files.delete(a) }),
      readFile: vi.fn(async (name: string) => {
        if (String(name).endsWith('recognition-catalog.json')) return JSON.stringify(payload)
        if (files.has(name)) return files.get(name)
        throw new Error('ENOENT')
      }),
    }))
    vi.useFakeTimers(); vi.setSystemTime(new Date('2026-10-10T12:00:00Z'))
    const fetcher = vi.fn(async (url: string) => new Response(JSON.stringify(url.includes('gtimg')
      ? { version: '16.20', hero: Array(173).fill({}) } : payload)))
    vi.stubGlobal('fetch', fetcher)
    const service = await import('../../src/main/services/recognition-data-service.ts')
    await service.loadRecognitionCatalogLocal()
    expect(fetcher).not.toHaveBeenCalled()
    expect(service.getRecognitionCatalog()?.entries.length).toBe(201)
    await service.refreshRecognitionSources()
    expect(service.getRecognitionDataStatus().referencePatch).toBe('16.20')
    const saved = service.getRecognitionCatalog()
    expect(write).toHaveBeenCalledTimes(2)
    await service.refreshRecognitionSources()
    expect(fetcher).toHaveBeenCalledTimes(2)
    vi.setSystemTime(new Date('2026-10-10T13:00:00Z'))
    fetcher.mockResolvedValue(new Response(JSON.stringify({ ...payload, rows: [] })))
    await service.refreshRecognitionSources()
    expect(service.getRecognitionCatalog()).toBe(saved)
    expect(service.getRecognitionDataStatus().providers[0]!.error).not.toBe('')
    expect(write).toHaveBeenCalledTimes(2)
  })
})
