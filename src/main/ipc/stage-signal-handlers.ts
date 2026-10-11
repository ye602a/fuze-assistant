import { trustedIpcMain } from '../security/trusted-ipc.ts'
import { StageSignalProbe } from '../services/stage-signal.ts'

export function registerStageSignalHandlers(): void {
  const probe = new StageSignalProbe()
  trustedIpcMain.handle('diagnostics-stage-signal', () => probe.inspect())
}
