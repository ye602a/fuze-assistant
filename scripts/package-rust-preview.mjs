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
const dir=path.join(root,'rust/artifacts/preview',`fuze-rust-preview-${revision}`)
await mkdir(dir,{recursive:true})
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
const files=[]
async function copy(source,destination){
 if(!destination || destination.split(/[\\/]/).some(x=>x==='..') || path.isAbsolute(destination))throw new Error('Invalid package path')
 if((await stat(source)).size>128*1024*1024)throw new Error('Oversize package input')
 const target=path.join(dir,destination);await mkdir(path.dirname(target),{recursive:true});await copyFile(source,target);files.push(destination)
}
async function text(name,body){await writeFile(path.join(dir,name),body);files.push(name)}
for(const file of ['fuze-rust-host.exe','fuze-ocr-worker.exe'])await copy(path.join(bin,file),file)
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
await text('Start-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nfuze-rust-host.exe --live-control preview.json 3600\r\npause\r\n')
await text('Check-preview.cmd','@echo off\r\ncd /d "%~dp0"\r\nfuze-rust-host.exe --check-bundle preview.json\r\npause\r\n')
await text('README.txt',`FUZE Rust 开发测试包 ${revision}\r\n\r\n这是独立开发预览，不是正式稳定版。请不要与稳定版同时识别。\r\n解压到普通目录，Check-preview.cmd 只进行离线自检；Start-preview.cmd 才会连接本机 LOL 并开始最长一小时的只读识别。关闭控制台可结束测试。\r\n默认自动识别语言和选择 OCR 后端；不会安装、开机自启、修改游戏或写入稳定版配置。\r\n本包没有正式主界面/托盘、快捷暂停、完整设置、在线更新、最新数据下载或战绩界面。使用随源码保存的数据，补丁新鲜度未保证。实际领取轮次未确认，推荐仅使用整体统计。\r\n本机游戏实战、独占全屏、多屏及权限兼容仍待验证。不要提交账号、令牌、游戏截图或原始日志。来源说明见 THANKS.md。\r\n`)
await text('build.json',JSON.stringify({revision,channel:'rust-development-preview',runtime:JSON.parse(await readFile(path.join(root,'node_modules/onnxruntime-node/package.json'),'utf8')).version,stable_replacement:false},null,2))
// Deliberately use another cwd and no repo runtime PATH: verifies relocation and DLL layout.
const child=spawnSync(path.join(dir,'fuze-rust-host.exe'),['--check-bundle',path.join(dir,'preview.json')],{cwd:path.dirname(dir),env:process.env,windowsHide:true,timeout:60000,encoding:'utf8',maxBuffer:65536})
let result;try{result=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!result?.bundle_ready||result.game_access!==false||!result.worker_reaped)throw new Error('Isolated bundle self-check failed')
const report={...result,revision,versions,files:files.length}
await writeFile(path.join(root,'rust/artifacts/preview-validation.json'),JSON.stringify(report,null,2))
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
