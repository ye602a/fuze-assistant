// Reproducible allowlist bundle; no installed-app data, credentials or capture paths.
import {mkdir,copyFile,writeFile,readFile,stat} from 'node:fs/promises'
import {execFileSync,spawnSync} from 'node:child_process'
import path from 'node:path'
import {createHash} from 'node:crypto'
import AdmZip from 'adm-zip'
import {inspectUpload} from './check-secrets.mjs'
import {prepare,root,ort} from './rust-prototype-tools.mjs'
import {playerPath as previewPath} from './rust-bundle-policy.mjs'
import {makeRuntimeContract} from './rust-runtime-contract.mjs'
const revision=execFileSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()
const version=(await readFile(path.join(root,'rust/app-version.txt'),'utf8')).trim()
if(!/^0\.7\.(0|[1-9]\d*)$/.test(version))throw new Error('Invalid application version')
if(!/^[a-f0-9]{12}$/.test(revision))throw new Error('Invalid revision')
const frontend=process.env.FUZE_UI_FRONTEND || 'slint'
if(!['win32','slint'].includes(frontend))throw new Error('Invalid UI frontend')
const previewRoot=path.resolve(process.env.FUZE_RUST_PREVIEW_DIR || path.join(root,'rust/artifacts/preview'))
const dir=path.join(previewRoot,`fuze-rust-preview-${revision}`)
await mkdir(previewRoot,{recursive:true})
// Never mix stale files or different frontends in an existing payload.
await mkdir(dir)
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
const onnxVersion=JSON.parse(await readFile(path.join(root,'node_modules/onnxruntime-node/package.json'),'utf8')).version
const runtimePolicyBytes=await readFile(path.join(root,'rust/vc-runtime-policy.json'))
const runtimeEvidence=JSON.parse(await readFile(path.join(bin,'fuze-build-evidence.json'),'utf8'))
const runtimeBinaries={}
const files=[]
async function copy(source,destination){
 if(!previewPath(destination))throw new Error('Invalid package path')
 if((await stat(source)).size>128*1024*1024)throw new Error('Oversize package input')
 const target=path.join(dir,destination);await mkdir(path.dirname(target),{recursive:true});await copyFile(source,target);files.push(destination)
}
async function text(name,body){if(!previewPath(name))throw new Error('Invalid package path');await writeFile(path.join(dir,name),body);files.push(name)}
for(const file of ['fuze-rust-host.exe','fuze-ocr-worker.exe','fuze-update.exe'])await copy(path.join(bin,file),file)
if(frontend==='slint')await copy(path.join(bin,'fuze-ui-lab.exe'),'FUZE.exe')
for(const file of ['onnxruntime.dll','DirectML.dll','dxcompiler.dll','dxil.dll'])await copy(path.join(path.dirname(ort),file),file)
// Inspect the copied bytes, not mutable source paths. Missing/stale build receipts block new packages.
for(const file of files)runtimeBinaries[file==='FUZE.exe'?'fuze-ui-lab.exe':file]=await readFile(path.join(dir,file))
const vcRuntime=makeRuntimeContract({policyBytes:runtimePolicyBytes,evidence:runtimeEvidence,revision,frontend,onnxVersion,binaries:runtimeBinaries})
const args=await prepare()
await copy(args[1],'resources/inference.onnx')
await copy(args[2],'resources/dictionary.json')
const versions={}
for(const locale of ['zh-CN','zh-TW','en-US']) {
 const pointer=locale==='zh-CN'?'current.json':`current.${locale}.json`
 const base='resources/client-data'
 const current=JSON.parse(await readFile(path.join(root,base,pointer),'utf8'))
 if(current.locale!==locale || !/^\d+(\.\d+){1,5}$/.test(current.dataVersion))throw new Error('Invalid bundle pointer')
 const prefix=`${base}/versions/${locale==='zh-CN'?'':locale+'/'}${current.dataVersion}`
 const manifest=JSON.parse(await readFile(path.join(root,prefix,'manifest.json'),'utf8'))
 if(manifest.locale!==locale || manifest.dataVersion!==current.dataVersion || !Array.isArray(manifest.files) || manifest.files.length>1024)throw new Error('Invalid bundle manifest')
 const selected=new Set(['manifest.json','augments.json','champions.json','items.json','champion-shards/index.json'])
 for(const entry of manifest.files) {
  const file=entry.path || entry.logicalPath
  if(typeof file==='string' && /^champion-shards\/[a-zA-Z0-9_-]+\.json$/.test(file))selected.add(file)
 }
 await copy(path.join(root,base,pointer),`${base}/${pointer}`)
 for(const file of selected)await copy(path.join(root,prefix,file),`${prefix}/${file}`)
 versions[locale]=current.dataVersion
}
await copy(path.join(root,'THANKS.md'),'THANKS.md')
await text('preview.json',JSON.stringify({bundled_data:'resources/client-data',locale:'auto',worker:'fuze-ocr-worker.exe',ort:'onnxruntime.dll',model:'resources/inference.onnx',dictionary:'resources/dictionary.json',backend:'auto'},null,2))
await text('README.txt',`FUZE ${version}（构建 ${revision}）

启动：双击 ${frontend === 'slint' ? 'FUZE.exe' : 'fuze-update.exe'}，或使用安装后的 FUZE 快捷方式。
安装：运行 Setup.exe，选择安装目录。安装不会自动启动程序。
便携使用：将 ZIP 完整解压到一个目录后启动，不要只复制 EXE。
所需 VC++ DLL 随程序提供。

更新：在设置中检查新版。下载期间可以继续使用，准备完成后退出、覆盖并重启。
卸载：使用 Windows“已安装的应用”中的 FUZE 卸载入口；便携版退出后删除其解压目录。
反馈：在设置的“帮助与反馈”中导出脱敏报告；截图保存在本地，不自动上传。
用户设置和记录独立保存，更新不清空用户文件。
来源与致谢见 THANKS.md。
`.replace(/\n/g,'\r\n'))
await text('build.json',JSON.stringify({version,revision,sequence:Number(execFileSync('git',['show','-s','--format=%ct','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()),uiFrontend:frontend,desktopExecutable:frontend==='slint'?'FUZE.exe':undefined,channel:'rust-development-preview',runtime:onnxVersion,vcRuntime,stable_replacement:false},null,2))
// Deliberately use another cwd and no repo runtime PATH: verifies relocation and DLL layout.
const child=spawnSync(path.join(dir,'fuze-rust-host.exe'),['--check-bundle',path.join(dir,'preview.json')],{cwd:path.dirname(dir),env:process.env,windowsHide:true,timeout:60000,encoding:'utf8',maxBuffer:65536})
let result;try{result=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!result?.bundle_ready||result.game_access!==false||!result.worker_reaped)throw new Error('Isolated bundle self-check failed')
if(frontend==='slint') {
 const smoke=spawnSync(path.join(dir,'FUZE.exe'),['--desktop-smoke',path.join(dir,'preview.json')],{cwd:path.dirname(dir),env:process.env,windowsHide:true,timeout:60000,encoding:'utf8',maxBuffer:65536})
 if(smoke.status!==0)throw new Error('Relocated Slint smoke failed')
}
const report={...result,revision,versions,frontend,slint_smoke:frontend==='slint',files:files.length}
await writeFile(path.join(previewRoot,'preview-validation.json'),JSON.stringify(report,null,2))
for(const file of files) {
 const findings=inspectUpload(file,await readFile(path.join(dir,file)))
 if(findings.length)throw new Error('Preview privacy allowlist check failed: '+file)
}
const zip=new AdmZip()
for(const file of files)zip.addLocalFile(path.join(dir,file),path.posix.dirname(file)==='.'?'':path.posix.dirname(file))
const archive=path.join(path.dirname(dir),`fuze-rust-preview-${revision}.zip`)
zip.writeZip(archive)
const checksum=createHash('sha256').update(await readFile(archive)).digest('hex')
await writeFile(archive+'.sha256',checksum+'  '+path.basename(archive)+'\n')
console.log(JSON.stringify({...report,archive:path.basename(archive),sha256:checksum}))
