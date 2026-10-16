// Reproducible allowlist bundle; no installed-app data, credentials or capture paths.
import {mkdir,copyFile,writeFile,readFile,stat} from 'node:fs/promises'
import {execFileSync,spawnSync} from 'node:child_process'
import path from 'node:path'
import {createHash} from 'node:crypto'
import AdmZip from 'adm-zip'
import {inspectUpload} from './check-secrets.mjs'
import {prepare,root,ort} from './rust-prototype-tools.mjs'
const revision=execFileSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()
if(!/^[a-f0-9]{12}$/.test(revision))throw new Error('Invalid revision')
const frontend=process.env.FUZE_UI_FRONTEND || 'win32'
if(!['win32','slint'].includes(frontend))throw new Error('Invalid UI frontend')
const previewRoot=path.resolve(process.env.FUZE_RUST_PREVIEW_DIR || path.join(root,'rust/artifacts/preview'))
const dir=path.join(previewRoot,`fuze-rust-preview-${revision}`)
await mkdir(previewRoot,{recursive:true})
// Never mix stale files or different frontends in an existing payload.
await mkdir(dir)
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
const files=[]
async function copy(source,destination){
 if(!destination || destination.split(/[\\/]/).some(x=>x==='..') || path.isAbsolute(destination))throw new Error('Invalid package path')
 if((await stat(source)).size>128*1024*1024)throw new Error('Oversize package input')
 const target=path.join(dir,destination);await mkdir(path.dirname(target),{recursive:true});await copyFile(source,target);files.push(destination)
}
async function text(name,body){await writeFile(path.join(dir,name),body);files.push(name)}
for(const file of ['fuze-rust-host.exe','fuze-ocr-worker.exe','fuze-update.exe'])await copy(path.join(bin,file),file)
if(frontend==='slint')await copy(path.join(bin,'fuze-ui-lab.exe'),'fuze-ui-lab.exe')
for(const file of ['onnxruntime.dll','DirectML.dll','dxcompiler.dll','dxil.dll'])await copy(path.join(path.dirname(ort),file),file)
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
await text('Start-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --launch\r\n')
await text('Check-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart /wait "" fuze-rust-host.exe --check-bundle preview.json\r\nif errorlevel 1 (echo Offline check failed.) else (echo Offline check passed.)\r\npause\r\n')
await text('Uninstall-local.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --uninstall-local\r\n')
await text('Install-local.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --install-local\r\n')
await text('Update.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --update\r\n')
await text('Private-channel.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --update-private\r\n')
await text('Forget-private-token.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --forget-credentials\r\n')
await text('Rollback.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --rollback\r\n')
await text('README.txt',`Start menu: Fuze Rust Preview / Uninstall Fuze Rust Preview. Uninstall-local.cmd removes managed program files and shortcuts only; settings, records, caches, tokens, unknown files and lock remain. The temporary uninstaller is left for system temp cleanup.\r\nIn-game reference: tray menu, supported active match only; independent of OCR. Match export supports JSON / PNG (local file, no upload).\r\nFUZE Rust 开发测试包 ${revision}\r\n\r\n这是独立开发预览，不替换稳定版。请不要与稳定版同时识别。\r\n可运行 Install-local.cmd 安装到独立用户目录，之后使用 LOCALAPPDATA/FuzeRustPreview/releases/Start-Fuze-Rust.cmd，不再依赖解压目录；登记到当前用户的 Windows 已安装应用，可从系统设置卸载，不自动启动。也可保持便携方式：解压后运行 Start-preview.cmd 。普通启动跟随上次自动识别开关，默认开启；暂停和继续会保存。离线检查不会连接游戏。\r\n暂停立即取消旧推荐，后台清理截图/OCR后显示已暂停；关闭窗口收起到托盘，退出按钮或托盘菜单结束程序。托盘不可用时关闭窗口直接退出。\r\n暂停状态可手动更新推荐数据。只下载公开数据，校验完整后启用；独立缓存为 LOCALAPPDATA/FuzeRustPreview/client-data，不修改稳定版。\r\n实际 OCR 后端由 worker 回报；连接、截图、语言错误分别显示。实际领取轮次未确认，推荐继续使用整体胜率。\r\n需要系统具备 x64 Visual C++ 运行库，当前包仍依赖 VCRUNTIME140.dll；不自动安装运行库。\r\n使用 Update.cmd 或暂停后的“退出并检查更新”检查 Rust 开发频道；公开仓库无需令牌。私有测试使用 Private-channel.cmd，令牌仅保存在 Windows 凭据管理器，Forget-private-token.cmd 可删除。下载与安装需分别确认，差分失败一次全量回退；版本在独立目录切换，下次通过 Start-preview.cmd 启动，失败自动回滚。发布端需要 rust-preview 预发布及 rust-update.json/.fuz/.blockmap 配套资产，不会读取 Electron latest.yml。\r\n原生偏好设置、战绩详情、个人记录与选人参考已有实现；写入类功能和实战验收仍未全部迁移。真实国服、独占全屏、多屏及长期运行仍待验证。\r\n不要提交账号、令牌、游戏截图或原始日志。来源说明见 THANKS.md。\r\n`)
await text('build.json',JSON.stringify({revision,sequence:Number(execFileSync('git',['show','-s','--format=%ct','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()),uiFrontend:frontend,channel:'rust-development-preview',runtime:JSON.parse(await readFile(path.join(root,'node_modules/onnxruntime-node/package.json'),'utf8')).version,stable_replacement:false},null,2))
// Deliberately use another cwd and no repo runtime PATH: verifies relocation and DLL layout.
const child=spawnSync(path.join(dir,'fuze-rust-host.exe'),['--check-bundle',path.join(dir,'preview.json')],{cwd:path.dirname(dir),env:process.env,windowsHide:true,timeout:60000,encoding:'utf8',maxBuffer:65536})
let result;try{result=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!result?.bundle_ready||result.game_access!==false||!result.worker_reaped)throw new Error('Isolated bundle self-check failed')
if(frontend==='slint') {
 const smoke=spawnSync(path.join(dir,'fuze-ui-lab.exe'),['--desktop-smoke',path.join(dir,'preview.json')],{cwd:path.dirname(dir),env:process.env,windowsHide:true,timeout:60000,encoding:'utf8',maxBuffer:65536})
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
