import {build} from 'esbuild'
import {createRequire} from 'node:module'
import {spawn,spawnSync} from 'node:child_process'
import {mkdir,writeFile} from 'node:fs/promises'
import {createWriteStream} from 'node:fs'
import {pipeline} from 'node:stream/promises'
import {Readable,Transform} from 'node:stream'
import {createHash} from 'node:crypto'
import path from 'node:path'
const require=createRequire(import.meta.url)
async function main() {
const root=path.resolve('build/private-update-live-'+Date.now());await mkdir(root,{recursive:true})
const auth=spawnSync('git',['credential','fill'],{input:'protocol=https\nhost=github.com\n\n',encoding:'utf8',env:{...process.env,GIT_TERMINAL_PROMPT:'0',GCM_INTERACTIVE:'Never'},timeout:15000})
const token=process.env.FUZE_UPDATE_TEST_ACCESS || /^password=(.+)$/m.exec(auth.stdout||'')?.[1]?.trim();if(!token)throw new Error('Local GitHub credential unavailable')
const api='https://api.github.com/repos/ye602a/fuze-assistant/releases'
async function get(url,accept='application/vnd.github+json',depth=0){
 const u=new URL(url);if(depth>5||u.protocol!=='https:'||!['api.github.com','release-assets.githubusercontent.com','objects.githubusercontent.com'].includes(u.hostname))throw new Error('Unexpected asset redirect')
 const response=await fetch(u,{headers:{Accept:accept,...(u.hostname==='api.github.com'?{Authorization:`Bearer ${token}`}:{})},redirect:'manual',signal:AbortSignal.timeout(120000)})
 if([301,302,307,308].includes(response.status)){await response.body?.cancel();return get(response.headers.get('location'),accept,depth+1)}
 if(!response.ok)throw new Error('Asset HTTP '+response.status)
 return response
}
const release=await(await get(api+'/tags/v0.4.2')).json()
const manifest=require('js-yaml').load(await(await get(release.assets.find(a=>a.name==='latest.yml').url,'application/octet-stream')).text())
const file=manifest.files.find(f=>f.url==='Fuze-Setup-0.4.2.exe'),asset=release.assets.find(a=>a.name===file.url)
const hash=createHash('sha512');let size=0
const meter=new Transform({transform(chunk,_enc,done){size+=chunk.length;hash.update(chunk);done(null,chunk)}})
console.log('Downloading an isolated 0.4.2 baseline; no installation.')
await pipeline(Readable.fromWeb((await get(asset.url,'application/octet-stream')).body),meter,createWriteStream(path.join(root,'baseline.exe')))
if(size!==file.size||hash.digest('base64')!==file.sha512)throw new Error('Baseline integrity mismatch')
await build({entryPoints:['tests/electron/private-update-live.ts'],bundle:true,platform:'node',format:'cjs',outfile:path.join(root,'live.cjs'),packages:'external'})
const env={...process.env,FUZE_UPDATE_TEST_ACCESS:token,FUZE_UPDATE_TEST_ROOT:root};delete env.ELECTRON_RUN_AS_NODE
const child=spawn(require('electron'),[path.join(root,'live.cjs')],{env,windowsHide:true,stdio:'inherit'})
const timer=setTimeout(()=>child.kill(),600000);const code=await new Promise((resolve,reject)=>{child.on('error',reject);child.on('exit',resolve)});clearTimeout(timer)
console.log(JSON.stringify({liveVerificationDirectory:root,success:code===0}));if(code!==0)process.exitCode=1

}
main().catch(() => { console.error('Live private update verification failed'); process.exitCode=1 })
