import { mkdtemp, mkdir, readFile, readdir, rm, symlink, utimes, writeFile } from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { StorageManager } from '../../src/main/services/storage-manager.ts'
import { getProtectedDataVersions, protectDataVersion, withDataMaintenance } from '../../src/main/data-maintenance.ts'
import type { StoragePreview } from '../../src/shared/ipc-contract.ts'

let root: string
let active = false
let manager: StorageManager
const idle = vi.fn(async () => !active)
const old = new Date(Date.now() - 9 * 86400000)
async function put(relative: string, body: unknown) {
  await mkdir(path.dirname(path.join(root, relative)), { recursive: true })
  await writeFile(path.join(root, relative), JSON.stringify(body))
}
async function age(relative: string) {
  for (const child of await readdir(path.join(root, relative), { withFileTypes: true })) {
    const name = `${relative}/${child.name}`
    if (child.isDirectory()) await age(name)
    await utimes(path.join(root, name), old, old)
  }
  await utimes(path.join(root, relative), old, old)
}
async function version(name: string, locale = 'zh-CN') {
  const prefix = `data/versions/${locale === 'zh-CN' ? '' : locale + '/'}${name}`
  const files = ['augments.json', 'champions.json', 'items.json', 'champion-shards/index.json']
  for (const file of files) await put(`${prefix}/${file}`, [])
  await put(`${prefix}/manifest.json`, { files: files.map(name => ({ path: name, bytes: 2 })) })
  await age(prefix)
}
async function preview(): Promise<StoragePreview> {
  const result = await manager.preview()
  expect(result.success).toBe(true)
  return (result as { success: true; preview: StoragePreview }).preview
}
beforeEach(async () => {
  root = await mkdtemp(path.join(os.tmpdir(), 'fuze-storage-test-'))
  active = false; idle.mockClear()
  manager = new StorageManager(root, idle, () => active)
  for (const name of ['1.0', '2.0', '3.0', '4.0']) await version(name)
  await put('data/current.json', { locale: 'zh-CN', dataVersion: '4.0' })
})
afterEach(async () => { vi.useRealTimers(); await rm(root, { recursive: true, force: true }) })

describe('storage retention and cleanup', () => {
  it('previews without deleting; removes only old versions and preserves current, fallback and personal files', async () => {
    for (const file of ['match-history/personal-history-v1.json', 'match-history/augment-journal-v1.json', 'logs/test.log', 'config.json', 'ocr-partial-screenshots/test.png']) await put(file, 'protected')
    const plan = await preview()
    expect(plan.candidates.map(c => c.label)).toEqual(['zh-CN · 1.0', 'zh-CN · 2.0'])
    expect(await readdir(path.join(root, 'data/versions'))).toHaveLength(4)
    expect(await manager.clean(plan.id)).toMatchObject({ success: true, removedFiles: 10, removedBytes: plan.reclaimableBytes })
    expect(await readdir(path.join(root, 'data/versions'))).toEqual(['3.0', '4.0'])
    expect(await readFile(path.join(root, 'config.json'), 'utf8')).toContain('protected')
    expect(await readdir(path.join(root, 'match-history'))).toHaveLength(2)
    expect((await preview()).candidates).toEqual([])
    expect(await manager.clean(plan.id)).toMatchObject({ success: false, issue: 'expired' })
  })
  it('blocks missing, expired and arbitrary-path requests', async () => {
    expect(await manager.clean('../logs')).toMatchObject({ success: false, issue: 'expired' })
    const plan = await preview()
    vi.spyOn(Date, 'now').mockReturnValue(plan.expiresAt + 1)
    expect(await manager.clean(plan.id)).toMatchObject({ success: false, issue: 'expired' })
    vi.restoreAllMocks()
  })
  it('rejects gameplay and a game starting after preview without deleting anything', async () => {
    active = true
    expect(await manager.preview()).toMatchObject({ success: false, issue: 'game-active' })
    active = false
    const plan = await preview()
    active = true
    expect(await manager.clean(plan.id)).toMatchObject({ success: false, issue: 'game-active', removedFiles: 0 })
  })
  it('revalidates changed files and newly activated versions', async () => {
    const plan = await preview()
    await put('data/versions/1.0/augments.json', [123])
    expect(await manager.clean(plan.id)).toMatchObject({ success: false, issue: 'changed', removedFiles: 0 })
    const second = await preview()
    await put('data/current.json', { dataVersion: '2.0', locale: 'zh-CN' })
    expect(await manager.clean(second.id)).toMatchObject({ success: false, issue: 'changed', removedFiles: 0 })
  })
  it('keeps unknown files and recently modified versions', async () => {
    await put('data/versions/1.0/notes.txt', 'keep')
    await age('data/versions/1.0')
    await put('data/versions/2.0/augments.json', [])
    expect((await preview()).candidates).toEqual([])
  })
  it('retains all versions when the current pointer or current data is incomplete', async () => {
    await put('data/current.json', { dataVersion: '../outside' })
    expect(await preview()).toMatchObject({ candidates: [], protectionUnknown: true })
    await put('data/current.json', { dataVersion: '4.0' })
    await rm(path.join(root, 'data/versions/4.0/items.json'))
    expect(await preview()).toMatchObject({ candidates: [], protectionUnknown: true })
  })
  it('keeps the newest complete fallback rather than an incomplete version', async () => {
    await rm(path.join(root, 'data/versions/3.0/items.json'))
    await age('data/versions/3.0')
    expect((await preview()).candidates.map(c => c.label)).toEqual(['zh-CN · 1.0', 'zh-CN · 3.0'])
  })
  it('handles language-scoped pointers independently', async () => {
    for (const name of ['1.0', '2.0', '3.0']) await version(name, 'en-US')
    await put('data/current.en-US.json', { dataVersion: '3.0', locale: 'en-US' })
    expect((await preview()).candidates.map(c => c.label)).toContain('en-US · 1.0')
  })
  it('only removes expired, recognizable recommendation temporary files', async () => {
    const stale = 'data/current.json.123.456.tmp'
    await put(stale, 'partial'); await utimes(path.join(root, stale), old, old)
    await put('data/current.json.123.789.tmp', 'recent')
    await put('data/other.tmp', 'unknown'); await utimes(path.join(root, 'data/other.tmp'), old, old)
    const plan = await preview()
    expect(plan.candidates.filter(c => c.kind === 'temporary').map(c => c.label)).toEqual(['current.json.123.456.tmp'])
    await manager.clean(plan.id)
    expect(await readFile(path.join(root, 'data/other.tmp'), 'utf8')).toContain('unknown')
  })
  it('does not traverse junctions or delete an old version containing one', async () => {
    await put('outside/keep.json', 'protected')
    await symlink(path.join(root, 'outside'), path.join(root, 'data/versions/1.0/link'), 'junction')
    await utimes(path.join(root, 'data/versions/1.0'), old, old)
    const plan = await preview()
    expect(plan.skipped).toBeGreaterThan(0)
    expect(plan.candidates.map(c => c.label)).not.toContain('zh-CN · 1.0')
    await manager.clean(plan.id)
    expect(await readFile(path.join(root, 'outside/keep.json'), 'utf8')).toContain('protected')
  })
  it('pins in-flight versions and waits for maintenance before allowing new version access', async () => {
    await version('0.9')
    await protectDataVersion('zh-CN', '0.9')
    expect(getProtectedDataVersions().has('zh-CN:0.9')).toBe(true)
    expect((await preview()).candidates.map(c => c.label)).not.toContain('zh-CN · 0.9')
    let finish!: () => void
    const held = withDataMaintenance(() => new Promise<void>(resolve => { finish = resolve }))
    const loaded = vi.fn()
    const request = protectDataVersion('en-US', '0.8').then(loaded)
    await Promise.resolve()
    expect(loaded).not.toHaveBeenCalled()
    finish(); await held; await request
    expect(loaded).toHaveBeenCalledOnce()
  })
})
