import {spawnSync,execFileSync} from 'node:child_process'
import {readFile,writeFile,mkdtemp,rm} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {root} from './rust-prototype-tools.mjs'
const revision=execFileSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()
const directory=path.join(root,'rust/artifacts/preview'),manifest=path.join(directory,'rust-update.json')
const m=JSON.parse(await readFile(manifest,'utf8'))
if(m.revision!==revision)throw new Error('Stale native update artifact')
const temp=await mkdtemp(path.join(os.tmpdir(),'fuze-native-update-'))
try{
 const child=spawnSync(path.join(directory,`fuze-rust-preview-${revision}`,'fuze-update.exe'),['--verify-package',manifest,path.join(directory,m.archive),path.join(temp,'version')],{cwd:temp,windowsHide:true,encoding:'utf8',timeout:90000,maxBuffer:65536})
 let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
 if(child.status!==0||!report?.package_verified||!report.native_extracted||report.game_access!==false)throw new Error('Native update package verification failed: '+String(child.stderr).replace(/[^a-z0-9-\s]/gi,'').slice(0,128))
 const verifier=path.join(process.env.FUZE_RUST_BIN_DIR || path.join(root,'rust/target/release'),'fuze-update-verify.exe')
 const lifecycle=spawnSync(verifier,['--offline',manifest,path.join(directory,m.archive),path.join(directory,`fuze-rust-preview-${revision}`),path.join(temp,'installation')],{cwd:temp,windowsHide:true,encoding:'utf8',timeout:180000,maxBuffer:65536,env:{...process.env,FUZE_RUST_CI_TRAY:'0'}})
 let installed;try{installed=JSON.parse(lifecycle.stdout?.trim())}catch{}
 if(lifecycle.status!==0||!installed?.local_setup||!installed.setup_idempotent||!installed.installed||!installed.repeat_launch||!installed.health_failure_rollback||!installed.process_start_failure_rollback||installed.game_access!==false)throw new Error('Native install/rollback lifecycle failed: '+String(lifecycle.stderr).replace(/[^a-z0-9-\s]/gi,'').slice(0,128))
 report={...report,...installed}
 await writeFile(path.join(root,'rust/artifacts/update-validation.json'),JSON.stringify(report,null,2));console.log(JSON.stringify(report))
}finally{if(path.resolve(temp).startsWith(path.resolve(os.tmpdir())+path.sep)&&path.basename(temp).startsWith('fuze-native-update-'))await rm(temp,{recursive:true,force:true})}
