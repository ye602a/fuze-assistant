export interface ConnectionStatus {
  discovered: boolean
  verifiedAt: number | null
  phase: string | null
  error: string | null
}

export interface RuntimeStatus {
  sampledAt: number
  version: string
  connection: ConnectionStatus
  clientDiscovery?: { processCount: number; directories: string[]; checkedAt: number | null }
  levelGate?: { mode: string; capture: boolean; level: number | null; nextLevel: number | null; checkedAt: number | null; skipped: number; watchdogMs: number } | null
  recognitionData?: { referencePatch: string; catalogPatch: string; catalogDate: string; count: number; attribution: string;
    strategy: string; providers: Array<{ checkedAt: number; updatedAt?: number; error: string; source: string; role: string }> }
  automatic: boolean
  capture: { error?: string | null; running: boolean; capturing: boolean; analyzing: boolean; count: number; analysisCount: number; lastCaptureAt: number | null; lastAnalysisMs: number }
  ocr: 'idle' | 'loading' | 'ready' | 'error'
  ocrExecution?: { backend: string; warming: boolean; fallback: boolean; reason: string }
  captureExecution?: { backend: string; frames: number; duplicates: number; error: string | null }
  data: { version: string; patch: string; state: 'loading' | 'ready' | 'error' }
  recent: { detectedAt: number | null; augments: Array<{ id: number | string | null; name: string; slot: number; missing: boolean; tier?: number | string | null }> }
  resources: { sampledAt: number; cpuPercent: number; workingSetMB: number; windows: number } | null
}

export function connectionIsVerified(connection: ConnectionStatus, now = Date.now()): boolean {
  return connection.discovered && !connection.error && connection.verifiedAt !== null &&
    now - connection.verifiedAt <= 20000 && now >= connection.verifiedAt
}

export function runtimeState(status: RuntimeStatus, now = Date.now()): string {
  if (!connectionIsVerified(status.connection, now)) return status.connection.discovered ? 'connectionError' : 'waiting'
  if (!status.automatic) return 'paused'
  if (status.data?.state === 'error') return 'dataError'
  if (status.connection.phase === 'InProgress') {
    if (!status.automatic) return 'paused'
    if (status.ocr === 'error') return 'ocrError'
    if (status.capture.error) return 'captureError'
    if (status.levelGate?.mode === 'saving') return 'savingGame'
    return status.capture.running ? 'inGame' : 'starting'
  }
  if (status.connection.phase === 'ChampSelect') return 'champSelect'
  if (status.connection.phase === 'GameStart') return 'loadingGame'
  return 'lobby'
}
