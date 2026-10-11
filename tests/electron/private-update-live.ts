import { app } from 'electron'
import { URL } from 'node:url'
import type { OutgoingHttpHeaders } from 'node:http'
import { RELEASE_API } from '../../src/main/update/request-policy.ts'
import { readFile, mkdir, copyFile, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { CancellationToken } from 'builder-util-runtime'
import { FuzeNsisUpdater } from '../../src/main/update/private-nsis-updater.ts'
import { FuzePrivateProvider } from '../../src/main/update/private-provider.ts'
class HistoricalPrivateProvider extends FuzePrivateProvider {
  protected override httpRequest(url: URL, headers?: OutgoingHttpHeaders | null, cancellation?: CancellationToken) {
    return super.httpRequest(url.href === RELEASE_API + '/latest' ? new URL(RELEASE_API + '/tags/v0.5.0') : url, headers, cancellation)
  }
}
async function main() {
  const base = path.resolve(process.env.FUZE_UPDATE_TEST_ROOT || '')
  const token = process.env.FUZE_UPDATE_TEST_ACCESS || ''
  delete process.env.FUZE_UPDATE_TEST_ACCESS
  if (!token || !process.env.FUZE_UPDATE_TEST_ROOT) throw new Error('Test setup missing')
  app.setPath('userData', path.join(base,'electron'))
  app.disableHardwareAcceleration()
  await app.whenReady()
  const config=path.join(base,'app-update.yml');await writeFile(config,JSON.stringify({updaterCacheDirName:'cache'}))
  await mkdir(path.join(base,'cache'),{recursive:true})
  await copyFile(path.join(base,'baseline.exe'),path.join(base,'cache','installer.exe'))
  const updater = new FuzeNsisUpdater({version:'0.4.2',name:'fixture',isPackaged:true,appUpdateConfigPath:config,userDataPath:base,baseCachePath:base,
    whenReady:async()=>{},relaunch(){throw new Error('Must not relaunch')},quit(){throw new Error('Must not install')},onQuit(){}})
  updater.logger=null;updater.autoDownload=false;updater.autoInstallOnAppQuit=false;updater.on('error',()=>{});updater.on('download-progress',()=>{})
  updater.on('private-update-fallback', result => console.log(JSON.stringify({privateUpdateFallback:result})))
  let bytes=0,requests=0
  const create=updater.httpExecutor.createRequest.bind(updater.httpExecutor)
  updater.httpExecutor.createRequest=(options,cb)=>{requests++;return create(options,response=>{response.on('data',(chunk:Buffer)=>{bytes+=chunk.length});cb(response)})}
  updater.setFeedURL({provider:'custom',updateProvider:HistoricalPrivateProvider,token})
  const latest=await updater.checkForUpdates()
  if(latest?.updateInfo.version!=='0.5.0')throw new Error('Unexpected live release')
  const started=Date.now();const files=await updater.downloadUpdate(new CancellationToken())
  const size=(await stat(files[0])).size
  if(size!==latest.updateInfo.files[0].size||bytes>=size)throw new Error('Differential transfer not established')
  const result={baseline:'0.4.2',target:'0.5.0',verifiedByUpdaterSha512:true,installed:false,downloadedBytesIncludingMetadata:bytes,fullInstallerBytes:size,requests,elapsedMs:Date.now()-started}
  await writeFile(path.join(base,'result.json'),JSON.stringify(result,null,2));console.log(JSON.stringify(result));app.exit(0)
}
main().catch(()=>{console.error('Live private differential verification failed; no installer executed.');app.exit(1)})
