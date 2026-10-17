// Reproducible allowlist bundle; no installed-app data, credentials or capture paths.
import {mkdir,copyFile,writeFile,readFile,stat} from 'node:fs/promises'
import {execFileSync,spawnSync} from 'node:child_process'
import path from 'node:path'
import {createHash} from 'node:crypto'
import AdmZip from 'adm-zip'
import {inspectUpload} from './check-secrets.mjs'
import {prepare,root,ort} from './rust-prototype-tools.mjs'
import {previewPath} from './rust-bundle-policy.mjs'
const revision=execFileSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()
if(!/^[a-f0-9]{12}$/.test(revision))throw new Error('Invalid revision')
const frontend=process.env.FUZE_UI_FRONTEND || 'slint'
if(!['win32','slint'].includes(frontend))throw new Error('Invalid UI frontend')
const previewRoot=path.resolve(process.env.FUZE_RUST_PREVIEW_DIR || path.join(root,'rust/artifacts/preview'))
const dir=path.join(previewRoot,`fuze-rust-preview-${revision}`)
await mkdir(previewRoot,{recursive:true})
// Never mix stale files or different frontends in an existing payload.
await mkdir(dir)
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
const files=[]
async function copy(source,destination){
 if(!previewPath(destination))throw new Error('Invalid package path')
 if((await stat(source)).size>128*1024*1024)throw new Error('Oversize package input')
 const target=path.join(dir,destination);await mkdir(path.dirname(target),{recursive:true});await copyFile(source,target);files.push(destination)
}
async function text(name,body){if(!previewPath(name))throw new Error('Invalid package path');await writeFile(path.join(dir,name),body);files.push(name)}
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
await copy(path.join(root,'scripts/export-rust-feedback.ps1'),'Export-feedback.ps1')
await copy(path.join(root,'docs/RUST_PLAYER_TEST_GUIDE.txt'),'TESTING-zh-CN.txt')
await text('Export-feedback.cmd','@echo off\r\ncd /d "%~dp0"\r\npowershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Export-feedback.ps1"\r\npause\r\n')
await text('preview.json',JSON.stringify({bundled_data:'resources/client-data',locale:'auto',worker:'fuze-ocr-worker.exe',ort:'onnxruntime.dll',model:'resources/inference.onnx',dictionary:'resources/dictionary.json',backend:'auto'},null,2))
await text('Start-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --launch\r\n')
// A targeted test must not silently route through a previously installed revision.
await text('Start-this-build.cmd','@echo off\r\ncd /d "%~dp0"\r\npowershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0Start-this-build.ps1"\r\n')
await text('Start-this-build.ps1',[
 "$ErrorActionPreference = 'Stop'",
 '$start = New-Object System.Diagnostics.ProcessStartInfo',
 "$start.FileName = Join-Path $PSScriptRoot '"+(frontend==='slint'?'fuze-ui-lab.exe':'fuze-rust-host.exe')+"'",
 "$start.Arguments = '--desktop preview.json'",
 '$start.WorkingDirectory = $PSScriptRoot',
 '$start.UseShellExecute = $false',
 '$start.CreateNoWindow = $true',
 '[void][System.Diagnostics.Process]::Start($start)',
 ''
].join('\r\n'))
await text('Check-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart /wait "" fuze-rust-host.exe --check-bundle preview.json\r\nif errorlevel 1 (echo Offline check failed.) else (echo Offline check passed.)\r\npause\r\n')
await text('Uninstall-local.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --uninstall-local\r\n')
await text('Install-local.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --install-local\r\n')
await text('Update.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --update\r\n')
await text('Private-channel.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --update-private\r\n')
await text('Forget-private-token.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --forget-credentials\r\n')
await text('Rollback.cmd','@echo off\r\ncd /d "%~dp0"\r\nstart "" fuze-update.exe --rollback\r\n')
await text('README.txt',`FUZE Rust 中文开发测试包 ${revision}

这是独立预览，保留已安装的稳定版。不要让两套助手同时识别。
启动、两局测试要点与脱敏反馈步骤统一见 TESTING-zh-CN.txt。
便携 ZIP 验证请运行 Start-this-build.cmd，直接启动当前解压版本；受管安装不包含此专用入口。
Start-preview.cmd 和开始菜单可能启动此前安装的受管版本。

安装与卸载
Install-local.cmd 安装到独立用户目录，登记到 Windows 已安装应用，不自动启动。
安装后可使用开始菜单 Fuze Rust Preview；目录为 %LOCALAPPDATA%/FuzeRustPreview/releases。
Uninstall-local.cmd 只删除受管程序与快捷方式，保留设置、记录、缓存、凭据及未知文件。
删除私有频道授权请单独运行 Forget-private-token.cmd。
当前载荷需要 Windows x64 和 Visual C++ x64 运行库，不会自动安装系统依赖。

数据与程序更新
暂停后可在设置中更新推荐数据，校验完成才启用；数据保存在独立缓存，不修改稳定版。
程序更新使用设置页的检查入口，或 Update.cmd；私有频道使用 Private-channel.cmd。
令牌仅保存在 Windows 凭据管理器。下载与安装分别确认，差分失败回退全量，失败安装支持回滚。
Rust 频道使用 rust-update.json、.fuz 和配套 .blockmap，不读取 Electron 的 latest.yml。

功能边界
界面为简体中文；自动推荐继续整体胜率，实际领取轮次仍未确认。
三卡分别显示第 1–4 次胜率参考，少样本或缺失会明确标注，不自动判断当前轮次。
阶段数据在选人/加载时准备，局内只读缓存；英雄详情也保留手动参考。
连接、截图和 OCR 状态会自动脱敏保存；退出后可运行 Export-feedback.cmd 导出反馈。
记录和分享均在本地处理。可在设置中配置独立凭据并开启每局 GitHub 脱敏反馈，默认关闭；不要发送整个数据目录、原始日志或账号信息。
真实游戏、跨屏、独占全屏及最终安装更新仍需验收，本包不代表完整迁移或正式稳定发布。
来源与致谢见 THANKS.md。
`.replace(/\n/g,'\r\n'))
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
