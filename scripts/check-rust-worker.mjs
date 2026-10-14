// Only committed offline fixtures and private children; no game/window discovery.
import path from 'node:path'
import { writeFile } from 'node:fs/promises'
import { prepare, output, childReport } from './rust-prototype-tools.mjs'
const args=await prepare()
const bin=process.env.FUZE_RUST_BIN_DIR || path.resolve('rust/target/release')
const host=path.join(bin,'fuze-worker-host.exe')
const lifecycle=childReport(host,['--lifecycle'])
const results=['cpu','auto'].map(mode=>({mode,...childReport(host,['--fixtures',path.join(bin,'fuze-ocr-worker.exe'),...args,mode])}))
const report={lifecycle,results,game_access:false,live_validated:false}
await writeFile(path.join(output,'worker-validation.json'),JSON.stringify(report,null,2))
console.log(JSON.stringify(report))
if(!lifecycle.exit_ok||!lifecycle.passed||results.some(r=>!r.exit_ok||!r.passed))process.exitCode=1
