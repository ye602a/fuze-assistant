import { StageStatisticsService } from '../services/stage-statistics.ts'
import { getAppDataDir } from '../modules/app-paths.ts'
import { trustedIpcMain } from '../security/trusted-ipc.ts'
import { getLCUServiceInstance } from '../services/lcu/lcu-service.ts'
import autoScreenshotService from '../auto-screenshot-service.ts'
import { getRecognitionDataStatus } from '../services/recognition-data-service.ts'

export function registerStageStatisticsHandlers(): void {
  const blocked = () => autoScreenshotService.getConfig().isRunning || ['GameStart', 'InProgress', 'Reconnect']
    .includes(getLCUServiceInstance().getConnectionStatus().phase || '')
  const service = new StageStatisticsService(getAppDataDir(), blocked)
  trustedIpcMain.handle('stage-statistics-get', async (_event, id: unknown, refresh: unknown) => ({
    ...await service.get(id, refresh), referencePatch: getRecognitionDataStatus().referencePatch,
  }))
}
