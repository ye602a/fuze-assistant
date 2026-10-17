// Entire production publisher -> native popup path with real OCR, never live game data.
import path from 'node:path'
import {mkdir,writeFile,readFile} from 'node:fs/promises'
import {prepare,root,childReport} from './rust-prototype-tools.mjs'
const args=await prepare()
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
const output=path.resolve(process.env.FUZE_SCREEN_REVIEW_DIR || path.join(root,'rust/artifacts/screen-review'))
await mkdir(output,{recursive:true})
// Freeze the historical image oracle so provider removals cannot silently reduce three-card coverage.
const fixtureData=path.join(output,'fixture-data'), version='1.0.0'
const versionDir=path.join(fixtureData,'versions',version)
await mkdir(path.join(versionDir,'champion-shards'),{recursive:true})
const catalog=JSON.parse(await readFile(args[3],'utf8'))
const augments=Object.entries(catalog).map(([name,id])=>({name,id}))
await writeFile(path.join(fixtureData,'current.json'),JSON.stringify({schemaVersion:3,locale:'zh-CN',dataVersion:version}))
await writeFile(path.join(versionDir,'manifest.json'),JSON.stringify({locale:'zh-CN',dataVersion:version,files:[{path:'augments.json'}]}))
await writeFile(path.join(versionDir,'augments.json'),JSON.stringify({data:augments}))
await writeFile(path.join(versionDir,'items.json'),JSON.stringify({data:[]}))
await writeFile(path.join(versionDir,'champion-shards/index.json'),JSON.stringify({dataVersion:version,shards:[{path:'champion-shards/1.json',championIds:[8]}]}))
await writeFile(path.join(versionDir,'champion-shards/1.json'),JSON.stringify({champions:{8:{augments:augments.map(a=>({...a,stats:{winRate:0.6,pickRate:0.12,games:900,source:'fixture',region:'TEST',gamePatch:'fixture'}}))}}}))
const ci=process.env.GITHUB_ACTIONS==='true' && process.env.FUZE_RUST_CI_VISIBLE==='1'
const modes=(process.env.FUZE_SCREEN_MODES || (ci?'screen,bitmap,offline-drop':'offline-stream,offline-bitmap,offline-drop')).split(',')
if(modes.some(m=>!['screen','bitmap','offline-stream','offline-bitmap','offline-drop'].includes(m)) ||
   (!ci && modes.some(m=>m==='screen'||m==='bitmap')))throw new Error('Screen capture is CI-only in this review')
const results=[]
const cases=modes.map(mode=>({mode,data:fixtureData,label:mode}))
if(process.env.FUZE_SCREEN_VERIFY_BUNDLED==='1')cases.push({mode:'offline-stream',data:path.join(root,'resources/client-data'),label:'bundled-current'})
for(const {mode,data,label} of cases) {
  const report=childReport(path.join(bin,'fuze-ui-lab.exe'),['--review-capture',
    path.join(bin,'fuze-ocr-worker.exe'),args[0],args[1],args[2],args[4],
    data,mode,'cpu',path.join(output,label)],120000)
  results.push(report)
  console.log(JSON.stringify({case:label,...report}))
  const completeStatistics = label==='bundled-current' || report.frames?.filter(f=>f.sample===0).every(f=>f.statistics_slots?.every(Boolean))
  if(!report.exit_ok || !report.passed || report.production_ui_publisher!==true ||
     !report.foreground_unchanged || report.hidden_native_popup_renders<2 || !report.worker_reaped || !completeStatistics) {
    process.exitCode=1
    break
  }
}
await writeFile(path.join(output,'screen-chain.json'),JSON.stringify({results,game_access:false,real_game_verified:false},null,2))
