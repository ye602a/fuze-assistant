import { readFile, mkdir, writeFile, rename } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { getAppDataDir } from '../modules/app-paths.ts'
import { parseMayhemCatalog, patchKey, type RecognitionCatalog } from '../../shared/recognition-catalog.ts'

const CATALOG_URL = 'https://arammayhem.com/data/v1/latest/augments.json'
const PATCH_URL = 'https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js'
const INTERVAL = 30 * 60 * 1000
let catalog: RecognitionCatalog | null = null
let referencePatch = ''
let initialized: Promise<void> | null = null
let refreshing: Promise<void> | null = null
let timer: ReturnType<typeof setInterval> | null = null
let lastAttempt = 0
let revision = 0
const status = {
  catalog: { checkedAt: 0, updatedAt: 0, error: '', source: 'arammayhem.com', role: 'recognition-popularity' },
  reference: { checkedAt: 0, error: '', source: 'Tencent', role: 'patch-reference' },
}
const root = () => path.join(getAppDataDir(), 'recognition-data')

async function publicJson(url: string): Promise<any> {
  // Only these fixed public origins; redirects never broaden the trust boundary.
  if (url !== CATALOG_URL && url !== PATCH_URL) throw new Error('Unapproved data URL')
  const response = await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(12000) })
  if (!response.ok || !response.body) throw new Error(`HTTP ${response.status}`)
  const reader = response.body.getReader(); const chunks: Uint8Array[] = []; let bytes = 0
  try {
    while (true) {
      const { value, done } = await reader.read()
      if (done) break
      bytes += value.length
      if (bytes > 2 * 1024 * 1024) { await reader.cancel(); throw new Error('Data exceeds size limit') }
      chunks.push(value)
    }
  } finally { reader.releaseLock() }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'))
}

async function atomicJson(name: string, value: unknown) {
  await mkdir(root(), { recursive: true })
  const target = path.join(root(), name)
  const temporary = `${target}.pending`
  await writeFile(temporary, JSON.stringify(value), 'utf8')
  await rename(temporary, target)
}

export function loadRecognitionCatalogLocal(): Promise<void> {
  if (!initialized) initialized = (async () => {
    const bundledCandidates = [
      ...(process.resourcesPath ? [path.join(process.resourcesPath, 'recognition-catalog.json')] : []),
      // electron-vite bundles this module into dist-electron/main.js in development.
      ...(!process.versions.electron || process.defaultApp ? [
        fileURLToPath(new URL('../../../resources/recognition-catalog.json', import.meta.url)),
        path.resolve('resources/recognition-catalog.json'),
      ] : []),
    ]
    // Raw public payloads are validated identically on disk and on the network.
    for (const file of [...new Set(bundledCandidates), path.join(root(), 'catalog.json')]) {
      try {
        const envelope = JSON.parse(await readFile(file, 'utf8'))
        const candidate = parseMayhemCatalog(envelope.payload ?? envelope, Number(envelope.retrievedAt) || 0)
        if (!catalog || compareCatalog(candidate, catalog) >= 0) { catalog = candidate; revision++ }
      } catch { /* Keep a complete known-good snapshot, including the bundled fallback. */ }
    }
    try {
      const data = JSON.parse(await readFile(path.join(root(), 'reference.json'), 'utf8'))
      if (patchKey(data.patch)) referencePatch = data.patch
    } catch { /* Offline reference can remain unknown. */ }
  })()
  return initialized
}

function compareCatalog(a: RecognitionCatalog, b: RecognitionCatalog) {
  return Number(patchKey(a.patch)?.split('.')[1]) - Number(patchKey(b.patch)?.split('.')[1]) || a.statsDate.localeCompare(b.statsDate)
}

export async function refreshRecognitionSources(): Promise<void> {
  if (refreshing) return refreshing
  if (Date.now() - lastAttempt < INTERVAL) return
  lastAttempt = Date.now()
  refreshing = (async () => {
    await loadRecognitionCatalogLocal()
    await Promise.allSettled([
      (async () => {
        status.catalog.checkedAt = Date.now()
        try {
          const payload = await publicJson(CATALOG_URL)
          const next = parseMayhemCatalog(payload, Date.now())
          if (catalog && compareCatalog(next, catalog) < 0) throw new Error('Provider returned an older snapshot')
          // Preserve the previous complete snapshot until validation and disk write succeed.
          await atomicJson('catalog.json', { retrievedAt: next.retrievedAt, payload })
          catalog = next; revision++; status.catalog.updatedAt = next.retrievedAt; status.catalog.error = ''
        } catch (error) { status.catalog.error = String((error as Error).message).slice(0, 120) }
      })(),
      (async () => {
        status.reference.checkedAt = Date.now()
        try {
          const payload = await publicJson(PATCH_URL)
          if (!patchKey(payload.version) || !Array.isArray(payload.hero) || payload.hero.length < 100) throw new Error('Invalid Tencent reference')
          await atomicJson('reference.json', { patch: payload.version, checkedAt: Date.now() })
          referencePatch = payload.version; status.reference.error = ''
        } catch (error) { status.reference.error = String((error as Error).message).slice(0, 120) }
      })(),
    ])
  })().finally(() => { refreshing = null })
  return refreshing
}

export function startRecognitionUpdates() {
  if (timer) return
  void refreshRecognitionSources()
  timer = setInterval(() => { void refreshRecognitionSources() }, INTERVAL)
  timer.unref()
}

export function getRecognitionCatalog() { return catalog }
export function getRecognitionRevision() { return revision }
export function getRecognitionDataStatus() {
  return { referencePatch, catalogPatch: catalog?.patch || '', catalogDate: catalog?.statsDate || '',
    count: catalog?.entries.length || 0, revision, attribution: catalog?.attribution || '',
    strategy: 'single-statistics-primary' as const, providers: [ { ...status.catalog }, { ...status.reference } ] }
}
