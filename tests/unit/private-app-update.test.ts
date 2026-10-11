import { beforeEach, describe, expect, it, vi } from 'vitest'
const mock = vi.hoisted(() => ({
  values: new Map<string, unknown>(), events: new Map<string, (...args: any[]) => void>(),
  send: vi.fn(), check: vi.fn(), download: vi.fn(), install: vi.fn(), feed: vi.fn(), encrypt: vi.fn(), encryption: true,
}))
vi.mock('electron', () => ({
  app: { isPackaged: true, getVersion: () => '0.4.0' },
  BrowserWindow: { getAllWindows: () => [{ isDestroyed: () => false, webContents: { send: mock.send } }] },
  safeStorage: { isEncryptionAvailable: () => mock.encryption, encryptString: (s: string) => { mock.encrypt(s); return Buffer.from('encrypted-' + s) }, decryptString: (s: Buffer) => s.toString().slice(10) },
}))
vi.mock('electron-updater', () => ({ default: {
  autoUpdater: { on: (key: string, cb: (...args: any[]) => void) => mock.events.set(key, cb), setFeedURL: mock.feed, checkForUpdates: mock.check, downloadUpdate: mock.download, quitAndInstall: mock.install },
  CancellationToken: class { cancelled = false; cancel() { this.cancelled = true } },
} }))
vi.mock('../../src/main/update/private-nsis-updater.ts', () => ({ FuzeNsisUpdater: class { on = (key: string, cb: (...args: any[]) => void) => mock.events.set(key, cb); setFeedURL = mock.feed; checkForUpdates = mock.check; downloadUpdate = mock.download; quitAndInstall = mock.install } }))
vi.mock('../../src/main/modules/app-store.ts', () => ({ default: { get: (k: string) => mock.values.get(k), set: (k: string, v: unknown) => mock.values.set(k, v), delete: (k: string) => mock.values.delete(k) } }))
vi.mock('../../src/main/modules/window-manager.ts', () => ({ allowMainWindowClose: vi.fn() }))
vi.mock('../../src/main/screenshot.ts', () => ({ getLolGameStatus: async () => ({ processRunning: false }) }))
beforeEach(() => { vi.resetModules(); vi.clearAllMocks(); mock.values.clear(); mock.events.clear(); mock.encryption = true; delete process.env.FUZE_GITHUB_TOKEN; delete process.env.FUZE_PORTABLE })
async function setup() { const service = await import('../../src/main/app-update-service.ts'); service.initializeAppUpdateService({ isDev: false }); return service }
describe('private Fuze update channel', () => {
  it('requires private authorization and never starts a request on initialization', async () => {
    const s = await setup(); expect(s.getAppUpdateState().phase).toBe('auth-required')
    expect((await s.checkForAppUpdate()).success).toBe(false); expect(mock.check).not.toHaveBeenCalled()
    expect(s.shouldInstallDownloadedAppUpdateOnQuit()).toBe(false)
  })
  it('encrypts access, fixes the repository and never exposes access in renderer state', async () => {
    const s = await setup(); const secret = 'test_private_access_123'
    expect(s.setAppUpdateCredential(secret).success).toBe(true)
    expect(mock.encrypt).toHaveBeenCalledWith(secret)
    mock.check.mockResolvedValue({ updateInfo: { version: '0.4.1' } })
    await s.checkForAppUpdate()
    expect(mock.feed).toHaveBeenCalledWith({ provider: 'custom', updateProvider: expect.any(Function), token: secret })
    expect(s.getAppUpdateState()).toMatchObject({ phase: 'available', canCheck: true, canDownload: true, autoUpdateEnabled: false })
    expect(JSON.stringify(mock.send.mock.calls)).not.toContain(secret)
    expect(mock.download).not.toHaveBeenCalled()
  })
  it('fails closed when encryption is unavailable', async () => {
    const s = await setup(); mock.encryption = false
    expect(s.setAppUpdateCredential('test_token').error).toBe('encryption-unavailable'); expect(mock.values.size).toBe(0)
  })
  it('coalesces checks and rejects replacement access while busy', async () => {
    const s = await setup(); s.setAppUpdateCredential('test_token')
    let done!: (v: unknown) => void
    mock.check.mockImplementation(() => new Promise(resolve => { done = resolve }))
    const one = s.checkForAppUpdate(); const two = s.checkForAppUpdate()
    expect(s.setAppUpdateCredential('').error).toBe('busy')
    expect(mock.check).toHaveBeenCalledTimes(1)
    done({ updateInfo: { version: '0.4.0' } }); await Promise.all([one, two])
    expect(s.getAppUpdateState()).toMatchObject({ phase: 'not-available', canDownload: false })
  })
  it('sanitizes upstream errors that include headers or credentials', async () => {
    const s = await setup(); s.setAppUpdateCredential('test_token')
    mock.check.mockRejectedValue(new Error('HTTP 401 Authorization: Bearer sensitive_secret'))
    expect((await s.checkForAppUpdate()).error).toBe('access-or-release')
    expect(JSON.stringify(mock.send.mock.calls)).not.toContain('sensitive_secret')
  })
  it('blocks download and install in a match; only explicit install works afterwards', async () => {
    const s = await setup(); s.setAppUpdateCredential('test_token'); mock.check.mockResolvedValue({ updateInfo: { version: '0.4.1' } }); await s.checkForAppUpdate()
    s.setAppUpdateGamePhase('InProgress'); expect((await s.downloadAppUpdate()).success).toBe(false)
    s.setAppUpdateGamePhase('Lobby'); mock.download.mockImplementation(async () => mock.events.get('update-downloaded')?.({ version: '0.4.1' }))
    await s.downloadAppUpdate(); s.setAppUpdateGamePhase('ChampSelect')
    expect(s.getAppUpdateState().canInstall).toBe(false); expect((await s.installDownloadedAppUpdate()).success).toBe(false)
    expect(mock.install).not.toHaveBeenCalled(); expect(s.shouldInstallDownloadedAppUpdateOnQuit()).toBe(false)
    s.setAppUpdateGamePhase('Lobby'); await s.installDownloadedAppUpdate(); expect(mock.install).toHaveBeenCalledWith(false, true)
  })
  it('cancels a download when a game begins and waits for a manual retry', async () => {
    const s = await setup(); s.setAppUpdateCredential('test_token'); mock.check.mockResolvedValue({ updateInfo: { version: '0.4.1' } }); await s.checkForAppUpdate()
    let active: any; let reject!: (e: Error) => void
    mock.download.mockImplementation(token => { active = token; return new Promise((_, fail) => { reject = fail }) })
    const download = s.downloadAppUpdate(); s.setAppUpdateGamePhase('GameStart'); expect(active.cancelled).toBe(true)
    reject(new Error('cancelled')); await download; s.setAppUpdateGamePhase('Lobby')
    expect(s.getAppUpdateState().canDownload).toBe(true); expect(mock.download).toHaveBeenCalledTimes(1)
  })
  it('allows retry after integrity failure and does not install corrupt files', async () => {
    const s = await setup(); s.setAppUpdateCredential('test_token'); mock.check.mockResolvedValue({ updateInfo: { version: '0.4.1' } }); await s.checkForAppUpdate()
    mock.download.mockRejectedValue(new Error('sha512 checksum mismatch'))
    expect((await s.downloadAppUpdate()).error).toBe('integrity'); expect(s.getAppUpdateState().canInstall).toBe(false); expect(mock.install).not.toHaveBeenCalled()
  })
  it('does not install a portable build or accept downgrades and prereleases', async () => {
    const s = await setup(); process.env.FUZE_PORTABLE = '1'
    expect(s.getAppUpdateState().installationSupported).toBe(false)
    expect(s.isNewerRelease('0.4.0', '0.4.0-local.1')).toBe(true)
    expect(s.isNewerRelease('0.4.0', '0.4.1')).toBe(false)
    expect(s.isNewerRelease('0.5.0-beta.1', '0.4.0')).toBe(false)
  })
})
