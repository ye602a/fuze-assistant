// Pin every version touched this process, including in-flight reads/downloads.
// Retention can reclaim other versions; pins are only small locale/version strings.
const pinned = new Set<string>()
let barrier: Promise<void> | null = null
export async function protectDataVersion(locale: string, version: string): Promise<void> {
  while (barrier) await barrier
  pinned.add(`${locale}:${version}`)
}
export function getProtectedDataVersions(): ReadonlySet<string> { return new Set(pinned) }
export async function withDataMaintenance<T>(task: () => Promise<T>): Promise<T> {
  if (barrier) throw new Error('Storage maintenance busy')
  let release!: () => void
  barrier = new Promise<void>(resolve => { release = resolve })
  try { return await task() } finally { barrier = null; release() }
}
