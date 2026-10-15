import {spawnSync} from 'node:child_process'
import {readFile,writeFile} from 'node:fs/promises'
import path from 'node:path'
import {root} from './rust-prototype-tools.mjs'
const revision=spawnSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).stdout.trim()
const folder=path.join(root,'rust/artifacts/preview',`fuze-rust-preview-${revision}`)
await writeFile(path.join(root,'rust/artifacts/desktop.png'),Buffer.alloc(0))
const child=spawnSync(path.join(folder,'fuze-rust-host.exe'),['--desktop-smoke',path.join(folder,'preview.json')],{
 cwd:root,windowsHide:true,timeout:20000,encoding:'utf8',maxBuffer:65536,
 env:{...process.env,FUZE_DESKTOP_PREVIEW:path.join(root,'rust/artifacts/desktop.png')}
})
let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!report?.desktop_ready||!report.pause_resume_exit||!report.preview_rendered||report.game_access!==false)throw new Error('Native desktop lifecycle failed')
const png=await readFile(path.join(root,'rust/artifacts/desktop.png'))
if(png.length<5000)throw new Error('Native desktop render missing')
await writeFile(path.join(root,'rust/artifacts/desktop-validation.json'),JSON.stringify(report,null,2))
console.log(JSON.stringify(report))
