import {spawnSync} from 'node:child_process'
import path from 'node:path'
import {root} from './rust-prototype-tools.mjs'
const bin=process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release')
// Explicit build resources only; never the installed application's data directory.
const child=spawnSync(path.join(bin,'fuze-rust-host.exe'),['--prepare-data',path.join(root,'resources/client-data')],{cwd:root,windowsHide:true,timeout:360000,encoding:'utf8',maxBuffer:65536})
let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!report?.data_ready||report.game_access!==false){
 const category=/fuze-host: ([a-z0-9-]+)/.exec(child.stderr || '')?.[1] || 'data-prepare-failed'
 throw new Error(category)
}
console.log(JSON.stringify(report))
