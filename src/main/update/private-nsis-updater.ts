import { stat, unlink } from 'node:fs/promises'
import { gunzipSync } from 'node:zlib'
import path from 'node:path'
import { CancellationError, type CancellationToken } from 'builder-util-runtime'
import { NsisUpdater } from 'electron-updater'
import type { DownloadUpdateOptions } from 'electron-updater/out/AppUpdater.js'
import type { AppAdapter } from 'electron-updater/out/AppAdapter.js'
import type { ResolvedUpdateFileInfo } from 'electron-updater/out/types.js'
import type { Provider } from 'electron-updater/out/providers/Provider.js'
import { PrivateUpdateExecutor } from './private-update-executor.ts'
import { reconstructPrivateUpdate, validateBlockmap } from './private-reconstruction.ts'

// electron-updater 6.8.9 exposes this runtime field but omits it from its generated declarations.
export class FuzeNsisUpdater extends NsisUpdater {
  httpExecutor: PrivateUpdateExecutor
  constructor(adapter?: AppAdapter) {
    super(null, adapter)
    this.httpExecutor = new PrivateUpdateExecutor()
    this.disableWebInstaller = true
  }
  override async downloadUpdate(token?: CancellationToken): Promise<string[]> {
    if (!token) throw new Error('Explicit update cancellation token required')
    this.httpExecutor.beginDownload(token)
    try { return await super.downloadUpdate(token) }
    finally { this.httpExecutor.endDownload() }
  }
  protected override async differentialDownloadInstaller(file: ResolvedUpdateFileInfo, options: DownloadUpdateOptions,
    destination: string, provider: Provider<any>, oldName: string): Promise<boolean> {
    if (options.cancellationToken.cancelled) throw new CancellationError()
    const baseline = this.downloadedUpdateHelper && path.join(this.downloadedUpdateHelper.cacheDir, oldName)
    if (this.downloadedUpdateHelper) {
      await unlink(path.join(this.downloadedUpdateHelper.cacheDirForPendingUpdate, 'current.blockmap')).catch((error: NodeJS.ErrnoException) => {
        if (error.code !== 'ENOENT') throw new Error('Update cache unavailable')
      })
    }
    // Avoid metadata/blockmap requests and a second hash pass when no baseline exists.
    const baselineInfo = baseline && await stat(baseline).catch(() => null)
    if (!baseline || !baselineInfo || !baselineInfo.isFile()) return true
    let step = 'release-metadata'
    try {
      const urls = await provider.getBlockMapFiles(file.url, this.currentVersion.version, options.updateInfoAndProvider.info.version)
      const load = async (url: URL, size: number) => {
        const data = await this.httpExecutor.downloadToBuffer(url, {
          headers: options.requestHeaders, cancellationToken: options.cancellationToken,
        })
        return validateBlockmap(JSON.parse(gunzipSync(data, { maxOutputLength: 8 * 1024 * 1024 }).toString()), size)
      }
      step = 'old-blockmap'
      const oldMap = await load(urls[0], baselineInfo.size)
      step = 'new-blockmap'
      const newMap = await load(urls[1], file.info.size!)
      step = 'reconstruction'
      await reconstructPrivateUpdate({ oldMap, newMap, oldFile: baseline, destination, url: file.url,
        sha512: file.info.sha512, size: file.info.size!, executor: this.httpExecutor, headers: options.requestHeaders,
        token: options.cancellationToken, onProgress: progress => this.emit('download-progress', progress) })
      return false
    } catch (error) {
      if (options.cancellationToken.cancelled) throw new CancellationError()
      const message = error instanceof Error ? error.message : ''
      const reason = /checksum mismatch/i.test(message) ? 'integrity' :
        /Invalid update blockmap/.test(message) ? 'blockmap' :
        /Update range size mismatch/.test(message) ? 'range-size' :
        /Update baseline truncated/.test(message) ? 'baseline-size' : 'unavailable'
      // Fixed categories only: never expose the SDK's URLs, headers or response bodies.
      this._logger.warn(`Private update fallback: ${step}/${reason}`)
      this.httpExecutor.resetRedirects()
      return true
    }
  }
}
