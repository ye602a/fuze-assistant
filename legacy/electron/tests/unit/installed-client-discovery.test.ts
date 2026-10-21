import { describe, expect, it, vi } from 'vitest'
import { InstalledClientDiscovery, normalizeInstallCandidates } from '../../src/main/services/lcu/installed-client-discovery.ts'
import type { TokenLoadResult } from '../../src/main/services/lcu/types.ts'

describe('bounded installation metadata fallback', () => {
  it('normalizes Chinese client and TCLS paths without following shares or URLs', () => {
    expect(normalizeInstallCandidates(['G:\\WeGameApps\\英雄联盟\\TCLS\\Client.exe', 'g:\\wegameapps\\英雄联盟',
      '\\\\server\\share', 'https://example.test/game', 'LeagueClient.exe'])).toEqual(['G:\\WeGameApps\\英雄联盟'])
  })
  it('rejects stale auth, tries the next install, coalesces requests and bounds rescans', async () => {
    let now = 10000
    const collect = vi.fn(async () => ['C:\\stale', 'G:\\current'])
    const read = vi.fn(async (directory: string): Promise<TokenLoadResult> => [directory.endsWith('current') ? 'good' : 'stale', '50000', null])
    const verify = vi.fn(async (auth: TokenLoadResult) => auth[0] === 'good')
    const discovery = new InstalledClientDiscovery(collect, read, verify, () => now)
    const first = discovery.discover(), second = discovery.discover()
    expect(first).toBe(second)
    expect(await first).toEqual(['good', '50000', null])
    expect(discovery.verifiedDirectory).toBe('G:\\current')
    expect(await discovery.discover()).toEqual([null, null, null])
    now += 31000
    await discovery.discover()
    expect(collect).toHaveBeenCalledOnce()
    expect(read.mock.calls.at(-1)).toEqual(['G:\\current'])
    verify.mockResolvedValue(false)
    now += 31000
    expect(await discovery.discover()).toEqual([null, null, null])
    expect(discovery.verifiedDirectory).toBeNull()
  })
})
