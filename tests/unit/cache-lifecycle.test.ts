import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { BoundedCache } from '../../src/main/bounded-cache.ts'
import { writeTextFileAtomic } from '../../src/main/atomic-data-write.ts'

const failure = vi.hoisted(() => ({ rename: false, write: false }))
vi.mock('node:fs/promises', async importOriginal => {
  const actual = await importOriginal<typeof import('node:fs/promises')>()
  return { ...actual,
    writeFile: async (...args: Parameters<typeof actual.writeFile>) => {
      await actual.writeFile(...args)
      if (failure.write) throw new Error('simulated disk failure')
    },
    rename: async (...args: Parameters<typeof actual.rename>) => {
      if (failure.rename) throw new Error('simulated locked target')
      return actual.rename(...args)
    },
  }
})
let directory: string | undefined
afterEach(async () => {
  failure.rename = false; failure.write = false
  if (directory) await rm(directory, { recursive: true, force: true })
  directory = undefined
})

describe('data cache lifecycle', () => {
  it('bounds thousands of versioned entries while retaining recently used data', () => {
    const cache = new BoundedCache<string, object>(256)
    const active = { version: 'current' }
    cache.set('active', active)
    for (let i = 0; i < 10_000; i++) {
      expect(cache.get('active')).toBe(active)
      cache.set(`old-version-${i}`, { i })
      expect(cache.size).toBeLessThanOrEqual(256)
    }
    expect(cache.has('old-version-0')).toBe(false)
    expect(cache.has('old-version-9999')).toBe(true)
  })
  it.each(['rename', 'write'] as const)('preserves the last good file and removes temporary files after repeated %s failures', async stage => {
    directory = await mkdtemp(path.join(os.tmpdir(), 'fuze-cache-test-'))
    const target = path.join(directory, 'data.json')
    await writeFile(target, 'known-good')
    failure[stage] = true
    for (let i = 0; i < 30; i++) await expect(writeTextFileAtomic(target, 'incomplete')).rejects.toThrow('simulated')
    expect(await readFile(target, 'utf8')).toBe('known-good')
    expect(await readdir(directory)).toEqual(['data.json'])
    failure[stage] = false
    await writeTextFileAtomic(target, 'new-complete')
    expect(await readFile(target, 'utf8')).toBe('new-complete')
    expect(await readdir(directory)).toEqual(['data.json'])
  })
})
