import { StorageManager } from '../services/storage-manager.ts'
import { getAppDataDir } from '../modules/app-paths.ts'
import { trustedIpcMain } from '../security/trusted-ipc.ts'
import { isLolGameProcessRunning } from '../screenshot.ts'
import autoScreenshotService from '../auto-screenshot-service.ts'
import { getLCUServiceInstance } from '../services/lcu/lcu-service.ts'

export function registerStorageIpcHandlers(): void {
  const busy = () => autoScreenshotService.getConfig().isRunning || ['ChampSelect', 'GameStart', 'InProgress', 'Reconnect']
    .includes(getLCUServiceInstance().getConnectionStatus().phase || '')
  const manager = new StorageManager(getAppDataDir(), async () => !busy() && await isLolGameProcessRunning() === false, busy)
  trustedIpcMain.handle('storage-preview', () => manager.preview())
  trustedIpcMain.handle('storage-clean', (_event, id: unknown) => manager.clean(id))
}
