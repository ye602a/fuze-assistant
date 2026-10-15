import {spawnSync} from 'node:child_process'
import {readFile,writeFile} from 'node:fs/promises'
import path from 'node:path'
import {root} from './rust-prototype-tools.mjs'
const revision=spawnSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).stdout.trim()
const folder=path.join(root,'rust/artifacts/preview',`fuze-rust-preview-${revision}`)
await writeFile(path.join(root,'rust/artifacts/desktop.png'),Buffer.alloc(0))
const child=spawnSync(path.join(folder,'fuze-rust-host.exe'),['--desktop-smoke',path.join(folder,'preview.json')],{
 cwd:root,windowsHide:true,timeout:20000,encoding:'utf8',maxBuffer:65536,
 env:{...process.env,FUZE_WORKBENCH_REVIEW:path.join(root,'rust/artifacts/workbench-review'),FUZE_DESKTOP_PREVIEW:path.join(root,'rust/artifacts/desktop.png'),FUZE_CHAMPION_PREVIEW:path.join(root,'rust/artifacts/champion-select.png'),FUZE_SIDE_PREVIEW:path.join(root,'rust/artifacts/augment-reference.png'),FUZE_STAGE_PREVIEW:path.join(root,'rust/artifacts/stage-reference.png'),FUZE_SETTINGS_PREVIEW:path.join(root,'rust/artifacts/settings.png'),FUZE_HISTORY_PREVIEW:path.join(root,'rust/artifacts/history.png'),FUZE_JOURNAL_PREVIEW:path.join(root,'rust/artifacts/journal.png')}
})
const sidePng=await readFile(path.join(root,'rust/artifacts/augment-reference.png'))
if(sidePng.length<1024||!sidePng.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10])))throw new Error('Side reference render failed')
let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!report?.desktop_ready||!report.pause_resume_exit||!report.preview_rendered||!report.champion_preview||report.game_access!==false)throw new Error('Native desktop lifecycle failed')
const png=await readFile(path.join(root,'rust/artifacts/desktop.png'))
if(png.length<5000)throw new Error('Native desktop render missing')
if((await readFile(path.join(root,'rust/artifacts/stage-reference.png'))).length<5000)throw new Error('Stage reference render missing')
report.stage_reference_preview=true
report.augment_reference_preview=true
// Additional locales exercise the same owned, hidden windows; never read user preferences.
report.locales=['zh-CN']
for (const locale of ['zh-TW','en-US']) {
 const prefix=path.join(root,'rust/artifacts',`desktop-${locale}`)
 const localized=spawnSync(path.join(folder,'fuze-rust-host.exe'),['--desktop-smoke',path.join(folder,'preview.json')],{
  cwd:root,windowsHide:true,timeout:20000,encoding:'utf8',maxBuffer:65536,
  env:{...process.env,FUZE_SMOKE_LOCALE:locale,FUZE_DESKTOP_PREVIEW:prefix+'.png',FUZE_CHAMPION_PREVIEW:prefix+'-champions.png',FUZE_SIDE_PREVIEW:prefix+'-augments.png',FUZE_STAGE_PREVIEW:prefix+'-stages.png',FUZE_SETTINGS_PREVIEW:prefix+'-settings.png',FUZE_RECORDS_PREVIEW:prefix+'-records.png',FUZE_HISTORY_PREVIEW:prefix+'-history.png',FUZE_JOURNAL_PREVIEW:prefix+'-journal.png'}
 })
 let value;try{value=JSON.parse(localized.stdout?.trim())}catch{}
 if(localized.status!==0||!value?.desktop_ready||!value.pause_resume_exit||!value.preview_rendered||!value.champion_preview||value.game_access!==false)throw new Error(`Native desktop locale failed: ${locale}`)
 report.locales.push(locale)
}
// Same two-step protocol as managed launch, but hidden and isolated from all game/user IO.
const health=spawnSync(path.join(folder,'fuze-rust-host.exe'),['--managed-desktop-smoke',path.join(folder,'preview.json')],{
 cwd:root,windowsHide:true,timeout:20000,encoding:'utf8',maxBuffer:65536,env:{...process.env,FUZE_RUST_CI_TRAY:'0'}
})
const lines=health.stdout?.trim().split(/\r?\n/)
let healthy;try{healthy=JSON.parse(lines?.[2])}catch{}
if(health.status!==0||lines?.[0]!=='FUZE_DESKTOP_READY_2'||lines?.[1]!=='FUZE_DESKTOP_HEALTHY_2'||healthy?.game_access!==false||!healthy?.pause_resume_exit)throw new Error('Managed UI health protocol failed')
report.managed_ui_health=true
await writeFile(path.join(root,'rust/artifacts/desktop-validation.json'),JSON.stringify(report,null,2))
console.log(JSON.stringify(report))
