import {spawnSync} from 'node:child_process'
import path from 'node:path'
import {pathToFileURL} from 'node:url'
import {root} from './rust-prototype-tools.mjs'
// A public CDN outage must not turn a whole build into an unbounded retry loop.
// Only transient transport failures retry; validation/auth/HTTP failures fail closed.
export async function prepareData(run,{clock=Date.now,pause=ms=>new Promise(r=>setTimeout(r,ms))}={}) {
 const deadline=clock()+360000
 for(let attempt=1;attempt<=3;attempt++) {
  const remaining=deadline-clock()
  if(remaining<=0)throw new Error('data-prepare-timeout')
  const child=run(remaining)
  let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
  if(child.status===0&&report?.data_ready&&report.game_access===false)return {...report,attempts:attempt}
  const category=/fuze-host: ([a-z0-9-]+)/.exec(child.stderr || '')?.[1] || 'data-prepare-failed'
  const delay=attempt===1?2000:5000
  if(category!=='update-network'||attempt===3||clock()+delay>=deadline)throw new Error(category)
  await pause(delay)
 }
 throw new Error('data-prepare-failed')
}
if(process.argv[1]&&import.meta.url===pathToFileURL(path.resolve(process.argv[1])).href){
 const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
 // Explicit build resources only; never the installed application's data directory.
 const report=await prepareData(timeout=>spawnSync(path.join(bin,'fuze-rust-host.exe'),['--prepare-data',path.join(root,'resources/client-data')],{cwd:root,windowsHide:true,timeout,encoding:'utf8',maxBuffer:65536}))
 console.log(JSON.stringify(report))
}
