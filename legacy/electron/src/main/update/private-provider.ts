import type { AppUpdater } from 'electron-updater'
import type { ProviderRuntimeOptions } from 'electron-updater/out/providers/Provider.js'
import { PrivateGitHubProvider, type PrivateGitHubUpdateInfo, type Asset } from 'electron-updater/out/providers/PrivateGitHubProvider.js'
import type { CustomPublishOptions } from 'builder-util-runtime'
import { URL } from 'node:url'
import { RELEASE_API, updateUrl } from './request-policy.ts'


const versionPattern = /^\d+\.\d+\.\d+$/

export class FuzePrivateProvider extends PrivateGitHubProvider {
  private latest: PrivateGitHubUpdateInfo | null = null
  constructor(options: CustomPublishOptions, updater: AppUpdater, runtime: ProviderRuntimeOptions) {
    super({ provider: 'github', owner: 'ye602a', repo: 'fuze-assistant', private: true }, updater, options.token,
      { ...runtime, isUseMultipleRangeRequest: false })
  }
  override async getLatestVersion() {
    const info = await super.getLatestVersion()
    if (!versionPattern.test(info.version)) throw new Error('Invalid update version')
    this.resolveFiles(info)
    this.latest = info
    return info
  }
  override resolveFiles(info: PrivateGitHubUpdateInfo) {
    const files = super.resolveFiles(info)
    if (files.length !== 1 || info.files?.[0]?.url !== `Fuze-Setup-${info.version}.exe` ||
      !/^[A-Za-z0-9+/]{86}==$/.test(files[0].info.sha512) || !Number.isSafeInteger(files[0].info.size) ||
      Number(files[0].info.size) <= 0 || Number(files[0].info.size) > 1024 * 1024 * 1024 || ('packages' in info && info.packages)) {
      throw new Error('Invalid update manifest')
    }
    files.forEach(file => this.assetUrl(file.url.href))
    return files
  }
  private assetUrl(value: string): URL {
    const url = updateUrl(value)
    if (!url.href.startsWith(RELEASE_API + '/assets/')) throw new Error('Invalid update asset')
    return url
  }
  private blockmap(assets: Asset[], version: string): URL {
    const matches = assets.filter(asset => asset.name === `Fuze-Setup-${version}.exe.blockmap`)
    if (matches.length !== 1) throw new Error('Blockmap unavailable')
    return this.assetUrl(matches[0].url)
  }
  override async getBlockMapFiles(base: URL, oldVersion: string, newVersion: string): Promise<URL[]> {
    if (!versionPattern.test(oldVersion) || !versionPattern.test(newVersion) || this.latest?.version !== newVersion ||
      !this.resolveFiles(this.latest).some(file => file.url.href === base.href)) throw new Error('Invalid differential baseline')
    const next = this.blockmap(this.latest.assets, newVersion)
    const raw = await this.httpRequest(new URL(`${RELEASE_API}/tags/v${oldVersion}`), {
      ...this.fileExtraDownloadHeaders, accept: 'application/vnd.github+json',
    })
    if (!raw || raw.length > 2 * 1024 * 1024) throw new Error('Invalid baseline release')
    const old = JSON.parse(raw)
    if (old.tag_name !== `v${oldVersion}` || old.draft || old.prerelease || !Array.isArray(old.assets) || old.assets.length > 100) {
      throw new Error('Invalid baseline release')
    }
    return [this.blockmap(old.assets, oldVersion), next]
  }
}
