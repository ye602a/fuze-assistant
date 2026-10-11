import { trustedIpcMain } from '../security/trusted-ipc.ts'
import { StageSignalProbe } from '../services/stage-signal.ts'
import { getLCUServiceInstance } from '../services/lcu/lcu-service.ts'

export function registerStageSignalHandlers(): void {
  const probe = new StageSignalProbe(undefined, Date.now, () => getLCUServiceInstance().getStageDiagnosticContext())
  trustedIpcMain.handle('diagnostics-stage-signal', () => probe.inspect())
}
