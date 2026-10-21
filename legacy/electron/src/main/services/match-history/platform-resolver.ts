type RecordValue = Record<string, unknown>
const record = (value: unknown): RecordValue => value && typeof value === 'object' ? value as RecordValue : {}

export function normalizeHistoryPlatform(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const platform = value.trim().toUpperCase()
  return /^[A-Z0-9]{2,16}$/.test(platform) && !['UNKNOWN', 'TENCENT', 'CN'].includes(platform) ? platform : null
}

export function historyPlatformFromPayload(value: unknown): string | null {
  const data = record(value)
  return normalizeHistoryPlatform(data.currentPlatformId) || normalizeHistoryPlatform(data.platformId)
}

export function historyPlatformFromIssuer(value: unknown): string | null {
  if (typeof value !== 'string') return null
  try {
    const url = new URL(value)
    // Read only a routing hint; never use an issuer as a network destination.
    if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) return null
    const match = url.hostname.match(/^([a-z0-9]+)(?:-[a-z0-9]+)*\.lol\.qq\.com$/i)
    return match ? normalizeHistoryPlatform(match[1]) : null
  } catch { return null }
}

export async function resolveHistoryPlatform(
  read: (endpoint: string) => Promise<{ status: number; data: unknown } | null>,
): Promise<string | null> {
  for (const endpoint of ['/lol-rso-auth/v1/authorization', '/lol-chat/v1/me', '/entitlements/v1/token']) {
    try {
      const response = await read(endpoint)
      if (response?.status !== 200) continue
      const platform = endpoint === '/entitlements/v1/token'
        ? historyPlatformFromIssuer(record(response.data).issuer)
        : historyPlatformFromPayload(response.data)
      if (platform) return platform
    } catch { /* Another local source may still be available. */ }
  }
  return null
}
