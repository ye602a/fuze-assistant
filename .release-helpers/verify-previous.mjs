import {spawnSync} from 'node:child_process';
import {readFile} from 'node:fs/promises';
import path from 'node:path';
const directory=path.resolve('rust/artifacts/preview');
const manifest=JSON.parse(await readFile(path.join(directory,'rust-update.json'),'utf8'));
const previous=path.join(process.env.RUNNER_TEMP,'fuze-previous-updater');
const child=spawnSync(path.join(previous,'fuze-update.exe'),['--verify-package',path.join(directory,'rust-update.json'),path.join(directory,manifest.archive),path.join(previous,'verified-new-payload')],{encoding:'utf8',windowsHide:true,timeout:90000,maxBuffer:65536});
let report;try{report=JSON.parse(child.stdout?.trim())}catch{}
if(child.status!==0||!report?.package_verified||!report.native_extracted||report.game_access!==false)throw Error('previous-updater-compatibility-failed: '+child.status);
console.log(JSON.stringify({previousVersion:'0.7.4',...report}));
