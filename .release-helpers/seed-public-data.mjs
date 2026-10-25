// Verified public data seed and isolated previous updater for transport compatibility.
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
const root=process.cwd(),require=createRequire(path.join(root,'package.json')),AdmZip=require('adm-zip');
const url='https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/fuze-rust-preview-3da2f67965de.zip';
const response=await fetch(url,{signal:AbortSignal.timeout(120000)});
if(!response.ok)throw Error('Public data seed HTTP '+response.status);
const bytes=Buffer.from(await response.arrayBuffer());
if(bytes.length!==72733854||createHash('sha256').update(bytes).digest('hex')!=='be955f11641dab3178b5032a13b6b8ca792f8246de372f7635ae41fae2ec646c')throw Error('Public seed checksum mismatch');
const zip=new AdmZip(bytes),versions={};let files=0;
for(const entry of zip.getEntries()){
 const name=entry.entryName;
 if(entry.isDirectory||!name.startsWith('resources/client-data/'))continue;
 if(!/^resources\/client-data\/[a-zA-Z0-9_./-]+\.json$/.test(name)||name.split('/').includes('..'))throw Error('Unexpected data seed path');
 const target=path.join(root,name);await mkdir(path.dirname(target),{recursive:true});
 const data=entry.getData();await writeFile(target,data,{flag:'wx'});files++;
 if(/^resources\/client-data\/current(?:\.[a-zA-Z-]+)?\.json$/.test(name)){
  const pointer=JSON.parse(data);versions[pointer.locale]=pointer.dataVersion;
 }
}
if(Object.keys(versions).length!==3)throw Error('Incomplete public data seed');
console.log(JSON.stringify({publicDataSeed:true,files,versions}));

const previous=path.join(process.env.RUNNER_TEMP,'fuze-previous-updater');
await mkdir(previous,{recursive:true});
const runtime=JSON.parse(zip.readAsText('vc-runtime-files.json'));
for(const name of ['fuze-update.exe',...Object.keys(runtime)]) {
 if(!/^[a-zA-Z0-9_.-]+\.(exe|dll)$/.test(name))throw Error('Unexpected previous runtime');
 await writeFile(path.join(previous,name),zip.readFile(name),{flag:'wx'});
}
