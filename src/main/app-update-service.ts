import { app, BrowserWindow, safeStorage } from 'electron'
import electronUpdater from 'electron-updater'
import { FuzeNsisUpdater } from './update/private-nsis-updater.ts'
import { FuzePrivateProvider } from './update/private-provider.ts'
import store from './modules/app-store.ts'
import { allowMainWindowClose } from './modules/window-manager.ts'
import type { AppUpdateState } from '../shared/ipc-contract.ts'

const { CancellationToken } = electronUpdater
const autoUpdater = new FuzeNsisUpdater()
export const FUZE_RELEASE_URL = 'https://github.com/ye602a/fuze-assistant/releases'
const CREDENTIAL_KEY = 'privateUpdateCredential'
const BLOCKED_PHASES = new Set(['ChampSelect', 'GameStart', 'InProgress', 'Reconnect'])
let initialized = false
let enabled = false
let installing = false
let cleanup: (() => Promise<void> | void) | null = null
let checkPromise: Promise<Result> | null = null
let downloadPromise: Promise<Result> | null = null
let cancellation: InstanceType<typeof CancellationToken> | null = null
let state: AppUpdateState = {
  phase: 'disabled', currentVersion: app.getVersion(), latestVersion: '',
  manualDownloadUrl: FUZE_RELEASE_URL, autoUpdateEnabled: false,
  credentialConfigured: false, gamePhase: '', available: false, downloaded: false,
  progress: null, error: '', lastCheckedAt: '', downloadDeferred: false,
}
type Result = { success: boolean; data: AppUpdateState; error?: string }

function readCredential(): string {
  if (process.env.FUZE_GITHUB_TOKEN) return process.env.FUZE_GITHUB_TOKEN.trim()
  try {
    const encrypted = store.get(CREDENTIAL_KEY)
    return typeof encrypted === 'string' && safeStorage.isEncryptionAvailable()
      ? safeStorage.decryptString(Buffer.from(encrypted, 'base64')) : ''
  } catch { return '' }
}
const gameBlocked = () => BLOCKED_PHASES.has(String(state.gamePhase || ''))
const busy = () => Boolean(checkPromise || downloadPromise || installing)
export function getAppUpdateState(): AppUpdateState {
  const credentialConfigured = Boolean(readCredential())
  return { ...state, progress: state.progress ? { ...state.progress } : null,
    currentVersion: app.getVersion(), credentialConfigured,
    canCheck: enabled && credentialConfigured && !busy() && !state.downloaded,
    canDownload: enabled && credentialConfigured && !busy() && !!state.available && !state.downloaded && !gameBlocked(),
    canInstall: enabled && !!state.downloaded && !busy() && !gameBlocked(),
    installationSupported: enabled && process.env.FUZE_PORTABLE !== '1',
    autoUpdateEnabled: false,
  }
}
function publish(patch: Partial<AppUpdateState> = {}) {
  state = { ...state, ...patch }
  const snapshot = getAppUpdateState()
  for (const window of BrowserWindow.getAllWindows()) {
    if (!window.isDestroyed()) window.webContents.send('app-update-status-changed', snapshot)
  }
  return snapshot
}
function result(success = true, error?: string): Result { return { success, data: getAppUpdateState(), ...(error ? { error } : {}) } }

// Upstream errors can contain request headers. Only these codes cross IPC or logs.
function classifyError(error: unknown): string {
  const text = error instanceof Error ? error.message : String(error || '')
  if (/401|403|404|unauthorized|forbidden/i.test(text)) return 'access-or-release'
  if (/sha512|checksum|digest/i.test(text)) return 'integrity'
  if (/timeout|timed out/i.test(text)) return 'timeout'
  return 'network'
}

export function initializeAppUpdateService(options: { isDev: boolean }): void {
  enabled = app.isPackaged && !options.isDev
  if (!initialized) {
    initialized = true
    autoUpdater.autoDownload = false
    autoUpdater.autoInstallOnAppQuit = false
    autoUpdater.allowPrerelease = false
    autoUpdater.allowDowngrade = false
    autoUpdater.logger = null
    autoUpdater.on('download-progress', progress => {
      if (!cancellation || cancellation.cancelled) return
      publish({ phase: 'downloading', progress: {
      percent: Math.min(100, Math.max(0, Number(progress.percent) || 0)), bytesPerSecond: Number(progress.bytesPerSecond) || 0,
    } })
    })
    autoUpdater.on('update-downloaded', event => {
      if (!cancellation || cancellation.cancelled) return
      publish({ phase: 'downloaded', latestVersion: event.version,
      downloaded: true, available: true, progress: { percent: 100 }, error: '' })
    })
    autoUpdater.on('error', () => {})
  }
  publish({ phase: enabled ? (readCredential() ? 'idle' : 'auth-required') : 'disabled' })
}

export function setAppUpdateCredential(value: unknown): Result {
  if (busy()) return result(false, 'busy')
  if (typeof value !== 'string' || value.length > 512 || (value.trim() && !/^[A-Za-z0-9_]+$/.test(value.trim()))) return result(false, 'invalid-credential')
  if (value.trim()) {
    if (!safeStorage.isEncryptionAvailable()) return result(false, 'encryption-unavailable')
    store.set(CREDENTIAL_KEY, safeStorage.encryptString(value.trim()).toString('base64'))
  } else store.delete(CREDENTIAL_KEY)
  publish({ phase: enabled ? (readCredential() ? 'idle' : 'auth-required') : 'disabled',
    available: false, downloaded: false, latestVersion: '', progress: null, error: '' })
  return result()
}

export async function refreshAppUpdateConfig(_options = {}): Promise<Result> {
  publish({ phase: enabled ? (readCredential() ? 'idle' : 'auth-required') : 'disabled' })
  return result()
}

export async function checkForAppUpdate(_reason = 'manual'): Promise<Result> {
  if (checkPromise) return checkPromise
  if (!enabled || downloadPromise || installing || state.downloaded) return result(false, 'unavailable')
  const token = readCredential()
  if (!token) { publish({ phase: 'auth-required' }); return result(false, 'auth-required') }
  autoUpdater.setFeedURL({ provider: 'custom', updateProvider: FuzePrivateProvider, token })
  publish({ phase: 'checking', error: '', available: false, latestVersion: '', progress: null })
  checkPromise = (async () => {
    try {
      const update = await autoUpdater.checkForUpdates()
      const latestVersion = update?.updateInfo.version || ''
      if (!latestVersion) throw new Error('Empty release')
      const available = isNewerRelease(latestVersion, app.getVersion())
      publish({ phase: available ? 'available' : 'not-available', latestVersion, available, error: '', lastCheckedAt: new Date().toISOString() })
      return result()
    } catch (error) {
      const code = classifyError(error)
      publish({ phase: 'error', error: code, available: false, lastCheckedAt: new Date().toISOString() })
      return result(false, code)
    }
  })().finally(() => { checkPromise = null; publish() })
  return checkPromise
}

export function isNewerRelease(latest: string, current: string): boolean {
  const a = /^(\d+)\.(\d+)\.(\d+)$/.exec(latest)
  const b = /^(\d+)\.(\d+)\.(\d+)(?:-([\w.-]+))?$/.exec(current)
  if (!a || !b) return false
  for (let i = 1; i <= 3; i++) { if (Number(a[i]) !== Number(b[i])) return Number(a[i]) > Number(b[i]) }
  return Boolean(b[4])
}

export async function downloadAppUpdate(_reason = 'manual-download'): Promise<Result> {
  if (downloadPromise) return downloadPromise
  if (!getAppUpdateState().canDownload || process.env.FUZE_PORTABLE === '1') return result(false, 'unavailable')
  cancellation = new CancellationToken()
  const active = cancellation
  publish({ phase: 'downloading', progress: { percent: 0 }, error: '', downloadDeferred: false })
  downloadPromise = (async () => {
    try { await autoUpdater.downloadUpdate(active); return result() }
    catch (error) {
      const code = active.cancelled ? '' : classifyError(error)
      publish({ phase: active.cancelled ? 'available' : 'error', error: code, progress: null, downloadDeferred: active.cancelled })
      return result(active.cancelled, code || undefined)
    }
  })().finally(() => { cancellation = null; downloadPromise = null; publish() })
  return downloadPromise
}

export function setAppUpdateGamePhase(phase: string | null | undefined): AppUpdateState {
  state.gamePhase = String(phase || '')
  if (gameBlocked() && cancellation) cancellation.cancel()
  return publish()
}
export function setAppUpdateInstallCleanup(callback: () => Promise<void> | void) { cleanup = callback }
export function isAppUpdateInstallInProgress() { return installing }
export function shouldInstallDownloadedAppUpdateOnQuit() { return false }
export async function installDownloadedAppUpdate(_reason = 'manual-install'): Promise<Result> {
  if (!getAppUpdateState().canInstall || process.env.FUZE_PORTABLE === '1') return result(false, 'unavailable')
  installing = true
  publish({ phase: 'installing', error: '' })
  try {
    const { getLolGameStatus } = await import('./screenshot.ts')
    const game = await getLolGameStatus()
    if (game.processRunning !== false || gameBlocked()) throw new Error('game-status-unavailable')
    await cleanup?.()
    if (gameBlocked()) throw new Error('game-active')
    allowMainWindowClose()
    autoUpdater.quitAndInstall(false, true)
    return result()
  } catch {
    installing = false
    publish({ phase: 'downloaded', error: 'install-failed' })
    return result(false, 'install-failed')
  }
}
