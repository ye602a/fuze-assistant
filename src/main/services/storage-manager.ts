import { lstat, readdir, readFile, realpath, rmdir, unlink } from 'node:fs/promises'
import path from 'node:path'
import { randomUUID } from 'node:crypto'
import type { StorageCategory, StoragePreview, StorageResult } from '../../shared/ipc-contract.ts'
import { getProtectedDataVersions, withDataMaintenance } from '../data-maintenance.ts'

const WEEK = 7 * 86400000
const VERSION = /^\d{1,4}(?:\.\d{1,4}){1,3}$/
const TEMP = /\.json\.(?:[a-f0-9-]{36}|\d+\.\d+)\.tmp$/i
const groups: Record<StorageCategory, string[]> = {
  recommendations: ['data'], recognition: ['recognition-data'], history: ['match-history'],
  screenshots: ['ocr-partial-screenshots'], logs: ['logs'],
  browser: ['Cache', 'Code Cache', 'GPUCache', 'DawnGraphiteCache', 'DawnWebGPUCache'],
}
type Entry = { relative: string; bytes: number; mtime: number; ino: number; directory: boolean }
type Candidate = { relative: string; label: string; kind: 'version' | 'temporary'; entries: Entry[] }
type Snapshot = { preview: StoragePreview; candidates: Candidate[] }
class StorageIssue extends Error { constructor(readonly issue: Extract<StorageResult, { success: false }>['issue']) { super(issue) } }
const normalized = (value: string) => process.platform === 'win32' ? path.resolve(value).toLowerCase() : path.resolve(value)
const compareVersions = (a: string, b: string) => {
  const left = a.split('.').map(Number), right = b.split('.').map(Number)
  for (let i = 0; i < Math.max(left.length, right.length); i++) if ((left[i] || 0) !== (right[i] || 0)) return (left[i] || 0) - (right[i] || 0)
  return 0
}
const signature = (candidates: Candidate[]) => JSON.stringify(candidates)

/** Only explicit UI actions scan. No timer, network, hashing or renderer-provided paths. */
export class StorageManager {
  private snapshot: Snapshot | null = null
  private running = false
  private rootResolved = false
  constructor(private root: string, private readonly verifyIdle: () => Promise<boolean>,
    private readonly busy: () => boolean = () => false) {}

  private assertIdle() { if (this.busy()) throw new StorageIssue('game-active') }
  private async safeStat(relative: string) {
    this.assertIdle()
    const target = path.resolve(this.root, relative)
    const within = path.relative(path.resolve(this.root), target)
    if (within.startsWith('..') || path.isAbsolute(within)) throw new StorageIssue('changed')
    const info = await lstat(target)
    if (info.isSymbolicLink() || normalized(await realpath(target)) !== normalized(target)) throw new StorageIssue('changed')
    return info
  }
  private async json(relative: string): Promise<any> {
    const info = await this.safeStat(relative)
    if (!info.isFile() || info.size > 1024 * 1024) throw new StorageIssue('changed')
    return JSON.parse(await readFile(path.join(this.root, relative), 'utf8'))
  }

  private async scan(): Promise<Snapshot> {
    if (!this.rootResolved) {
      // Windows TEMP/user paths may use short names or an aliased ancestor.
      // Pin the trusted root's physical path once; never follow links beneath it.
      const info = await lstat(this.root)
      if (!info.isDirectory() || info.isSymbolicLink()) throw new StorageIssue('changed')
      this.root = await realpath(this.root)
      this.rootResolved = true
    }
    const rootInfo = await this.safeStat('')
    if (!rootInfo.isDirectory()) throw new StorageIssue('scan-failed')
    const entries = new Map<string, Entry>()
    let skipped = 0, visited = 0
    const unsafe = new Set<string>()
    const walk = async (relative: string, depth = 0): Promise<void> => {
      if (++visited > 20000 || depth > 12) throw new StorageIssue('scan-failed')
      let info
      try { info = await this.safeStat(relative) } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return
        if (error instanceof StorageIssue && error.issue === 'game-active') throw error
        skipped++; unsafe.add(relative); return
      }
      if (!info.isDirectory() && !info.isFile()) { skipped++; unsafe.add(relative); return }
      entries.set(relative, { relative, bytes: info.isFile() ? info.size : 0, mtime: info.mtimeMs, ino: info.ino, directory: info.isDirectory() })
      if (info.isDirectory()) {
        let children: string[]
        try { children = await readdir(path.join(this.root, relative)) } catch { skipped++; unsafe.add(relative); return }
        for (const child of children.sort()) await walk(`${relative}/${child}`, depth + 1)
      }
    }
    const categories: StoragePreview['categories'] = []
    for (const [kind, dirs] of Object.entries(groups) as Array<[StorageCategory, string[]]>) {
      for (const dir of dirs) await walk(dir)
      const files = [...entries.values()].filter(entry => !entry.directory && dirs.some(dir => entry.relative.startsWith(`${dir}/`)))
      categories.push({ kind, bytes: files.reduce((sum, entry) => sum + entry.bytes, 0), files: files.length })
    }
    const descendants = (relative: string) => [...entries.values()].filter(e => e.relative === relative || e.relative.startsWith(`${relative}/`))
    const safeTree = (relative: string) => ![...unsafe].some(value => value === relative || value.startsWith(`${relative}/`) || relative.startsWith(`${value}/`))
    const versions: Array<{ relative: string; locale: string; version: string; complete: boolean; removable: boolean }> = []
    for (const entry of entries.values()) {
      const match = /^data\/versions\/(?:(zh-CN|zh-TW|en-US)\/)?([^/]+)$/.exec(entry.relative)
      if (!entry.directory || !match || !VERSION.test(match[2]) || !safeTree(entry.relative)) continue
      let complete = false
      let removable = false
      try {
        const envelope = await this.json(`${entry.relative}/manifest.json`)
        const manifest = envelope.data ?? envelope
        if (!Array.isArray(manifest.files)) throw new Error('Missing file list')
        const required = new Map<string, number>(['manifest.json', 'augments.json', 'champions.json', 'items.json', 'champion-shards/index.json'].map(name => [name, 0]))
        for (const file of manifest.files) {
          if (typeof file.path !== 'string' || file.path.includes('..') || file.path.includes('\\') || file.path.startsWith('/')) throw new Error('Invalid manifest path')
          if (required.has(file.path) || file.path.startsWith('champion-shards/')) required.set(file.path, Number(file.bytes) || 0)
        }
        complete = [...required].every(([name, bytes]) => {
          const file = entries.get(`${entry.relative}/${name}`)
          return file && !file.directory && file.bytes > 0 && (!bytes || bytes === file.bytes)
        })
        const allowed = new Set(['manifest.json', ...manifest.files.map((file: any) => file.path)])
        removable = descendants(entry.relative).every(file => file.directory || allowed.has(file.relative.slice(entry.relative.length + 1)) || TEMP.test(file.relative))
      } catch { /* Unknown or incomplete versions never serve as a fallback. */ }
      versions.push({ relative: entry.relative, locale: match[1] || 'zh-CN', version: match[2], complete, removable })
    }
    const pinned = getProtectedDataVersions()
    const candidates: Candidate[] = []
    let protectionUnknown = false
    for (const locale of ['zh-CN', 'zh-TW', 'en-US']) {
      const local = versions.filter(version => version.locale === locale)
      if (!local.length) continue
      const pointers = locale === 'zh-CN' ? ['current.json', 'current.zh-CN.json'] : [`current.${locale}.json`]
      const current = new Set<string>()
      let invalid = false
      for (const name of pointers) {
        try {
          const pointer = await this.json(`data/${name}`)
          if (!VERSION.test(pointer.dataVersion) || (pointer.locale && pointer.locale !== locale)) throw new Error('Invalid pointer')
          current.add(pointer.dataVersion)
        } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') invalid = true }
      }
      if (invalid || !current.size || ![...current].every(version => local.some(v => v.version === version && v.complete))) {
        protectionUnknown = true; continue
      }
      const oldestCurrent = [...current].sort(compareVersions)[0]
      const fallback = local.filter(v => v.complete && compareVersions(v.version, oldestCurrent) < 0).sort((a, b) => compareVersions(b.version, a.version))[0]?.version
      for (const version of local) {
        if (!version.removable || compareVersions(version.version, oldestCurrent) >= 0 || version.version === fallback || pinned.has(`${locale}:${version.version}`)) continue
        const contents = descendants(version.relative)
        if (contents.some(entry => Date.now() - entry.mtime < WEEK)) continue
        candidates.push({ relative: version.relative, label: `${locale} · ${version.version}`, kind: 'version', entries: contents })
      }
    }
    for (const entry of entries.values()) {
      if (entry.directory || !entry.relative.startsWith('data/') || !TEMP.test(entry.relative) || Date.now() - entry.mtime < WEEK || !safeTree(entry.relative)) continue
      if (candidates.some(candidate => entry.relative.startsWith(`${candidate.relative}/`))) continue
      const version = versions.find(v => entry.relative.startsWith(`${v.relative}/`))
      if (version && pinned.has(`${version.locale}:${version.version}`)) continue
      candidates.push({ relative: entry.relative, label: path.posix.basename(entry.relative), kind: 'temporary', entries: [entry] })
    }
    candidates.sort((a, b) => a.relative.localeCompare(b.relative))
    if (candidates.length > 100) { skipped += candidates.length - 100; candidates.splice(100) }
    const checkedAt = Date.now()
    const preview: StoragePreview = { id: randomUUID(), checkedAt, expiresAt: checkedAt + 120000, categories, skipped, protectionUnknown,
      candidates: candidates.map(candidate => ({ label: candidate.label, kind: candidate.kind,
        files: candidate.entries.filter(e => !e.directory).length, bytes: candidate.entries.reduce((sum, e) => sum + e.bytes, 0) })),
      reclaimableBytes: candidates.reduce((sum, c) => sum + c.entries.reduce((bytes, e) => bytes + e.bytes, 0), 0) }
    return { preview, candidates }
  }

  async preview(): Promise<StorageResult> {
    if (this.running) return { success: false, issue: 'busy' }
    this.running = true; this.snapshot = null
    try {
      if (!await this.verifyIdle()) throw new StorageIssue('game-active')
      this.snapshot = await this.scan()
      return { success: true, preview: this.snapshot.preview }
    } catch (error) { return { success: false, issue: error instanceof StorageIssue ? error.issue : 'scan-failed' } }
    finally { this.running = false }
  }

  async clean(id: unknown): Promise<StorageResult> {
    if (this.running) return { success: false, issue: 'busy' }
    const previous = this.snapshot
    this.snapshot = null
    if (!previous || typeof id !== 'string' || id !== previous.preview.id || Date.now() < previous.preview.checkedAt || Date.now() >= previous.preview.expiresAt) return { success: false, issue: 'expired' }
    this.running = true
    let removedBytes = 0, removedFiles = 0
    try {
      if (!await this.verifyIdle()) throw new StorageIssue('game-active')
      return await withDataMaintenance(async () => {
        const current = await this.scan()
        if (signature(current.candidates) !== signature(previous.candidates)) throw new StorageIssue('changed')
        // Revalidate all candidates before the first deletion; paths only come from this service.
        for (const candidate of current.candidates) for (const entry of candidate.entries) {
          const info = await this.safeStat(entry.relative)
          if (info.ino !== entry.ino || info.mtimeMs !== entry.mtime || (info.isFile() && info.size !== entry.bytes)) throw new StorageIssue('changed')
        }
        if (!await this.verifyIdle()) throw new StorageIssue('game-active')
        for (const candidate of current.candidates) {
          for (const entry of candidate.entries.filter(e => !e.directory)) {
            const info = await this.safeStat(entry.relative)
            if (!info.isFile() || info.ino !== entry.ino || info.mtimeMs !== entry.mtime || info.size !== entry.bytes) throw new StorageIssue('changed')
            await unlink(path.join(this.root, entry.relative))
            removedBytes += entry.bytes; removedFiles++
          }
          for (const entry of candidate.entries.filter(e => e.directory).sort((a, b) => b.relative.length - a.relative.length)) {
            if (!(await this.safeStat(entry.relative)).isDirectory()) throw new StorageIssue('changed')
            await rmdir(path.join(this.root, entry.relative))
          }
        }
        return { success: true as const, removedBytes, removedFiles }
      })
    } catch (error) { return { success: false, issue: error instanceof StorageIssue ? error.issue : 'cleanup-failed', removedBytes, removedFiles } }
    finally { this.running = false }
  }
}
