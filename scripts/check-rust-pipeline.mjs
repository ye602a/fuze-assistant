// No application discovery: only committed offline fixtures and generated own-window snapshots.
import path from 'node:path'
import { writeFile } from 'node:fs/promises'
import { prepare,output,run,runOcr } from './rust-prototype-tools.mjs'
const args=await prepare()
const adapters=run('fuze-capture',['--adapters'])
const hardware=adapters.exit_ok && adapters.adapters?.some(a=>a.index===0&&!a.software)
const results=[]
const old=process.env.FUZE_RUST_GPU_ATLAS
try {
  if (hardware) {
    process.env.FUZE_RUST_GPU_ATLAS='1'
    for (const mode of ['cpu','auto']) {
      const ocr=runOcr(args,mode)
      let overlay={exit_ok:false}
      const snapshot=ocr.samples?.[1]?.snapshot
      if (snapshot) {
        const file=path.join(output,'pipeline-snapshot.json')
        await writeFile(file,JSON.stringify(snapshot))
        overlay=run('fuze-native-overlay',['--smoke',path.join(output,'pipeline-overlay.png'),file])
      }
      results.push({mode,ocr,overlay})
    }
  }
} finally {
  if(old===undefined)delete process.env.FUZE_RUST_GPU_ATLAS
  else process.env.FUZE_RUST_GPU_ATLAS=old
}
let wgc={tested:false,reason:'visible-self-window-is-ci-only'}
const wgcAttempts=[]
if(process.env.GITHUB_ACTIONS==='true'&&process.env.FUZE_RUST_CI_VISIBLE==='1'){
  for(let attempt=0;attempt<5;attempt++){
    wgcAttempts.push(run('fuze-native-overlay',['--wgc-ci',path.join(output,'snapshot.json')]))
  }
  wgc=wgcAttempts.find(r=>!r.exit_ok)||wgcAttempts[0]
}
const report={hardware,results,wgc,wgcAttempts,gameAccess:false,gameValidated:false,encodedCaptureImages:0}
await writeFile(path.join(output,'pipeline-validation.json'),JSON.stringify(report,null,2))
console.log(JSON.stringify({hardware,results:results.map(({mode,ocr,overlay})=>({
  mode,passed:ocr.passed,backend:ocr.backend_status,frames:ocr.samples?.map(s=>({capture_ms:s.capture_ms,ocr_ms:s.runs_ms,roi_bytes:s.roi_bytes})),
  overlayPassed:overlay.exit_ok,error:ocr.error})),wgc}))
if(!adapters.exit_ok||results.some(r=>!r.ocr.exit_ok||!r.ocr.passed||!r.overlay.exit_ok)||
  (wgc.exit_ok===false)||wgcAttempts.some(r=>r.wgc_stream?.frames>0&&!r.wgc_stream?.verified))process.exitCode=1
