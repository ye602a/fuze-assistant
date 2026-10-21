import { app } from 'electron'
import assert from 'node:assert/strict'
import { mkdir, writeFile, readFile } from 'node:fs/promises'
import path from 'node:path'
import { tmpdir } from 'node:os'
import { createHash } from 'node:crypto'
import { gzipSync } from 'node:zlib'
import { CancellationToken } from 'builder-util-runtime'
import { FuzeNsisUpdater } from '../../src/main/update/private-nsis-updater.ts'
import { FuzePrivateProvider } from '../../src/main/update/private-provider.ts'
import { RELEASE_API } from '../../src/main/update/request-policy.ts'
import { session } from 'electron'
import fs from 'node:fs/promises'

async function main() {
const old = Buffer.alloc(8 * 1024 * 1024, 65), next = Buffer.from(old)
next.fill(66, 128 * 1024, 2 * 1024 * 1024)
const blockmap = (data: Buffer) => ({ version: '2', files: [{ name: 'file', offset: 0,
  sizes: Array.from({length: data.length / 65536}, () => 65536), checksums: Array.from({length:data.length / 65536}, (_,i) => createHash('sha256').update(data.subarray(i*65536,(i+1)*65536)).digest('base64')) }] })
const hashes = { old: gzipSync(JSON.stringify(blockmap(old))), next: gzipSync(JSON.stringify(blockmap(next))) }
const digest = createHash('sha512').update(next).digest('base64')
const asset = (id: number, name: string) => ({ url: `${RELEASE_API}/assets/${id}`, name })
const assets = [asset(1,'latest.yml'),asset(2,'Fuze-Setup-0.6.0.exe'),asset(3,'Fuze-Setup-0.6.0.exe.blockmap')]
const manifest = { version:'0.6.0', files:[{url:'Fuze-Setup-0.6.0.exe',sha512:digest,size:next.length}],path:'Fuze-Setup-0.6.0.exe',sha512:digest }
const fixtureAccess = 'fixture_access'
let scenario = '', rangeBytes = 0, fullBytes = 0, maps = 0, releaseCalls = 0, sawCdn = false
let cancelToken: CancellationToken
let copyBytesAfterCancel = 0, copyCancelled = false, copyStreams = 0
const realOpen = fs.open
// Test only: cancel at an actual COPY read, before and after a network range.
fs.open = async (...args: Parameters<typeof fs.open>) => {
  const handle = await realOpen(...args)
  if (String(args[0]).endsWith('installer.exe')) {
    copyStreams++
    handle.once('close', () => copyStreams--)
    const read = handle.read.bind(handle)
    handle.read = (async (...readArgs: Parameters<typeof handle.read>) => {
      const result = await read(...readArgs)
      if (cancelToken.cancelled) copyBytesAfterCancel += result.bytesRead
      if (!copyCancelled && (scenario === 'cancel-copy-first' || (scenario === 'cancel-copy-last' && rangeBytes > 0))) {
        copyCancelled = true
        cancelToken.cancel()
      }
      return result
    }) as typeof handle.read
  }
  return handle
}
const root = path.resolve(process.env.FUZE_UPDATE_FIXTURE_ROOT || '')
if(path.dirname(root)!==path.resolve(tmpdir())||!path.basename(root).startsWith('fuze-update-fixture-'))throw new Error('Unexpected fixture path')
app.setPath('userData',path.join(root,'electron'))
app.disableHardwareAcceleration()
await app.whenReady()
const partition = session.fromPartition('fuze-private-updater',{cache:false})
await partition.protocol.handle('https', async request => {
  const url = new URL(request.url)
  if (url.hostname === 'api.github.com') {
    assert.equal(request.headers.get('authorization'), `token ${fixtureAccess}`)
    assert.equal(request.headers.has('x-user-staging-id'), false)
    if (url.pathname.endsWith('/assets/2')&&scenario==='bad-redirect')return new Response(null,{status:302,headers:{location:'https://untrusted.invalid/file'}})
    if (url.pathname.endsWith('/latest')) return Response.json({assets})
    if (url.pathname.endsWith('/tags/v0.5.0')) {releaseCalls++;return Response.json({tag_name:'v0.5.0',assets:[asset(4,'Fuze-Setup-0.5.0.exe.blockmap')]})}
    assert.equal(request.headers.get('accept'), 'application/octet-stream')
    return new Response(null,{status:302,headers:{location:`https://release-assets.githubusercontent.com/${url.pathname.split('/').pop()}`}})
  }
  assert.equal(url.hostname,'release-assets.githubusercontent.com')
  assert.equal(request.headers.has('authorization'),false)
  assert.equal(request.headers.has('cookie'),false)
  sawCdn = true
  const id=url.pathname.slice(1)
  if(id==='1')return Response.json(manifest)
  if(id==='3'||id==='4') {
    maps++
    if(scenario==='bad-map'&&id==='3')return new Response('invalid gzip')
    if(scenario==='missing-map'&&id==='3')return new Response('missing',{status:404})
    return new Response(id==='3'?hashes.next:hashes.old)
  }
  assert.equal(id,'2')
  const range=request.headers.get('range')
  if(range) {
    const match=/^bytes=(\d+)-(\d+)$/.exec(range)!;assert.ok(match)
    const start=Number(match[1]),end=Number(match[2]);rangeBytes+=end-start+1
    assert.ok(end-start+1<=1024*1024)
    if(scenario==='cancel') {cancelToken.cancel();return new Response('cancelled',{status:503})}
    if(scenario==='ignore-range') {fullBytes+=next.length;return new Response(next)}
    const content=Buffer.from(next.subarray(start,end+1));if(scenario==='bad-integrity')content.fill(99)
    if(scenario==='short-range'||scenario==='long-range')return new Response(scenario==='short-range'?content.subarray(0,-1):Buffer.concat([content,Buffer.from([0])]),{status:206,headers:{'content-range':`bytes ${start}-${end}/${next.length}`}})
    return new Response(content,{status:206,headers:{'content-range':`bytes ${start}-${end}/${next.length}`}})
  }
  fullBytes+=next.length
  return new Response(scenario==='bad-integrity'?Buffer.alloc(next.length,99):next)
})
const results=[]
for(const kind of ['differential','missing-baseline','missing-map','bad-map','stale-baseline','ignore-range','short-range','long-range','bad-integrity','bad-redirect','cancel','cancel-copy-first','cancel-copy-last','retry']) {
  scenario=kind;rangeBytes=0;fullBytes=0;maps=0;releaseCalls=0;sawCdn=false;cancelToken=new CancellationToken()
  copyBytesAfterCancel=0;copyCancelled=false
  const base=path.join(root,kind);await mkdir(base,{recursive:true})
  const config=path.join(base,'app-update.yml');await writeFile(config,JSON.stringify({updaterCacheDirName:'cache'}))
  const cache=path.join(base,'cache');await mkdir(cache,{recursive:true})
  if(kind!=='missing-baseline')await writeFile(path.join(cache,'installer.exe'),kind==='stale-baseline'?Buffer.alloc(old.length,90):old)
  const updater=new FuzeNsisUpdater({version:'0.5.0',name:'fixture',isPackaged:true,appUpdateConfigPath:config,userDataPath:base,baseCachePath:base,
    whenReady:async()=>{},relaunch(){throw new Error('Must not relaunch')},quit(){throw new Error('Must not install')},onQuit(){}})
  updater.logger=null;updater.autoDownload=false;updater.autoInstallOnAppQuit=false;updater.on('error',()=>{});updater.on('download-progress',()=>{})
  const transferModes: Array<{mode: unknown; reason: unknown}> = []
  updater.onTransferMode = (mode,reason) => { transferModes.push({mode,reason}) }
  updater.setFeedURL({provider:'custom',updateProvider:FuzePrivateProvider,token:fixtureAccess})
  await updater.checkForUpdates()
  let files:string[]=[];let failed=false
  try{files=await updater.downloadUpdate(cancelToken)}catch{failed=true}
  if(kind.startsWith('cancel')||kind==='bad-integrity'||kind==='bad-redirect') {assert.ok(failed);assert.equal(updater.installerPath,null);if(kind.startsWith('cancel'))assert.equal(fullBytes,0)}
  else {assert.equal(failed,false,kind);assert.deepEqual(await readFile(files[0]),next);assert.ok(sawCdn)}
  if(kind==='differential'||kind==='retry'){assert.equal(fullBytes,0);assert.ok(rangeBytes>0&&rangeBytes<next.length)}
  if(kind==='missing-baseline'){assert.equal(maps,0);assert.equal(releaseCalls,0);assert.equal(fullBytes,next.length)}
  if(kind==='differential')assert.deepEqual(transferModes.at(-1),{mode:'differential',reason:''})
  if(kind==='missing-baseline')assert.deepEqual(transferModes.at(-1),{mode:'full',reason:'baseline-missing'})
  if(['missing-map','bad-map','stale-baseline','ignore-range'].includes(kind))assert.deepEqual(transferModes.at(-1),{mode:'full',reason:'differential-unavailable'})
  if(kind.startsWith('cancel'))assert.ok(!transferModes.some(value=>value.mode==='full'))
  if(['missing-map','bad-map','stale-baseline','ignore-range','short-range','long-range'].includes(kind))assert.ok(fullBytes>=next.length)
  results.push({scenario:kind,passed:true,rangeBytes,fullBytes,maps,releaseCalls})
  if(kind.startsWith('cancel-copy')) {
    assert.ok(copyCancelled);assert.equal(copyBytesAfterCancel,0);assert.equal(copyStreams,0)
    if(kind==='cancel-copy-first')assert.equal(rangeBytes,0)
  }
  if(kind.startsWith('cancel')) {
    scenario='same-instance-retry';rangeBytes=0;fullBytes=0
    const retry=await updater.downloadUpdate(new CancellationToken())
    assert.deepEqual(await readFile(retry[0]),next);assert.equal(fullBytes,0);assert.ok(rangeBytes>0)
    results.push({scenario,passed:true,rangeBytes,fullBytes,maps,releaseCalls})
  }
}
await writeFile(path.join(process.cwd(),'build/private-update-fixture-result.json'),JSON.stringify(results,null,2))
console.log(JSON.stringify({privateUpdateFixture:results}))
if(path.dirname(root)!==path.resolve(tmpdir())||!path.basename(root).startsWith('fuze-update-fixture-'))throw new Error('Unexpected fixture path')
app.exit(0)

}
main().catch(error => { console.error(error); app.exit(1) })
