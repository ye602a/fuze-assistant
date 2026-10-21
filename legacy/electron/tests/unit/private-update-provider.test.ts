import { describe, expect, it, vi } from 'vitest'
import { FuzePrivateProvider } from '../../src/main/update/private-provider.ts'
import { RELEASE_API, updateHeaders, updateUrl } from '../../src/main/update/request-policy.ts'
const asset = (id: number, name: string) => ({ url: `${RELEASE_API}/assets/${id}`, name })
const info = { version: '0.6.0', files: [{ url: 'Fuze-Setup-0.6.0.exe', size: 500, sha512: Buffer.alloc(64).toString('base64') }],
  assets: [asset(100, 'latest.yml'), asset(200, 'Fuze-Setup-0.6.0.exe'), asset(300, 'Fuze-Setup-0.6.0.exe.blockmap')] }
function setup(override = info) {
  const request = vi.fn(async (options: any) => {
    if (options.path.endsWith('/latest')) return JSON.stringify({ assets: override.assets })
    if (options.path.endsWith('/100')) return JSON.stringify(override)
    return JSON.stringify({ tag_name: 'v0.5.0', assets: [asset(400, 'Fuze-Setup-0.5.0.exe.blockmap')] })
  })
  const provider = new FuzePrivateProvider({ provider: 'custom', token: 'fixture_access' }, { allowPrerelease: false } as any,
    { executor: { request } as any, platform: 'win32', isUseMultipleRangeRequest: true })
  return { provider, request }
}
describe('private differential provider and credential boundaries', () => {
  it('resolves asset IDs by release and exact file name, not URL suffixes', async () => {
    const { provider, request } = setup()
    const latest = await provider.getLatestVersion()
    const maps = await provider.getBlockMapFiles(provider.resolveFiles(latest)[0].url, '0.5.0', '0.6.0')
    expect(maps.map(u => u.href)).toEqual([`${RELEASE_API}/assets/400`, `${RELEASE_API}/assets/300`])
    expect(request).toHaveBeenCalledTimes(3)
    expect(provider.isUseMultipleRangeRequest).toBe(false)
  })
  it('rejects remote paths, mismatched baselines and missing blockmaps', async () => {
    const { provider } = setup()
    const latest = await provider.getLatestVersion()
    await expect(provider.getBlockMapFiles(new URL('https://api.github.com/other'), '0.5.0', '0.6.0')).rejects.toThrow()
    latest.assets = latest.assets.filter(a => a.name !== 'Fuze-Setup-0.6.0.exe.blockmap')
    await expect(provider.getBlockMapFiles(provider.resolveFiles(latest)[0].url, '0.5.0', '0.6.0')).rejects.toThrow('Blockmap unavailable')
    expect(() => provider.resolveFiles({ ...info, assets: [asset(100, 'latest.yml'), {name: 'Fuze-Setup-0.6.0.exe',url: 'https://other.invalid/package.exe'}] })).toThrow()
  })
  it('never carries credentials, cookies, or staging identities to a CDN', () => {
    const headers = { Authorization: 'fixture_access', Cookie: 'fixture_cookie', 'X-User-Staging-Id': 'fixture_identity', Range: 'bytes=1-4', Accept: '*/*' }
    expect(updateHeaders(`${RELEASE_API}/assets/200`, headers)).toEqual({ authorization: 'fixture_access', range: 'bytes=1-4', accept: 'application/octet-stream' })
    expect(updateHeaders('https://release-assets.githubusercontent.com/a?signature=fixture', headers)).toEqual({ range: 'bytes=1-4', accept: '*/*' })
    for (const url of ['http://api.github.com/repos/ye602a/fuze-assistant/releases/latest', 'https://api.github.com/repos/other/repo/releases/latest', 'https://release-assets.githubusercontent.com.evil.invalid/a', 'https://x@release-assets.githubusercontent.com/a', 'https://release-assets.githubusercontent.com:444/a']) expect(() => updateUrl(url)).toThrow()
  })
})
