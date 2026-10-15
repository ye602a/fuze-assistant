// Only committed offline fixtures and private children; no game/window discovery.
import path from 'node:path'
import { writeFile, readFile, mkdir } from 'node:fs/promises'
import { prepare, output, childReport } from './rust-prototype-tools.mjs'
const args=await prepare()
// Public committed fixture data only; never uses the installed app's cache.
const dataset=path.join(output,'integrated-fixture-data')
const version='1.0.0'
const versionDir=path.join(dataset,'versions',version)
await mkdir(versionDir,{recursive:true})
const catalog=JSON.parse(await readFile(args[3],'utf8'))
await writeFile(path.join(dataset,'current.json'),JSON.stringify({schemaVersion:3,locale:'zh-CN',dataVersion:version}))
await writeFile(path.join(versionDir,'manifest.json'),JSON.stringify({locale:'zh-CN',dataVersion:version,files:[{path:'augments.json'}]}))
await writeFile(path.join(versionDir,'augments.json'),JSON.stringify({data:Object.entries(catalog).map(([name,id])=>({name,id}))}))
const bin=process.env.FUZE_RUST_BIN_DIR || path.resolve('rust/target/release')
const host=path.join(bin,'fuze-worker-host.exe')
const lifecycle=childReport(host,['--lifecycle'])
const results=['cpu','auto'].map(mode=>({mode,...childReport(host,['--fixtures',path.join(bin,'fuze-ocr-worker.exe'),...args,mode,dataset])}))
const report={lifecycle,results,game_access:false,live_validated:false}
await writeFile(path.join(output,'worker-validation.json'),JSON.stringify(report,null,2))
console.log(JSON.stringify(report))
if(!lifecycle.exit_ok||!lifecycle.passed||results.some(r=>!r.exit_ok||!r.passed))process.exitCode=1
