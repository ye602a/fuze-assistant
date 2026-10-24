// Build-time cache from an already public package. No executable is extracted.
import {mkdir,writeFile} from 'node:fs/promises';
import path from 'node:path';
import {createRequire} from 'node:module';
import {createHash} from 'node:crypto';
const root=process.cwd(),require=createRequire(path.join(root,'package.json')),AdmZip=require('adm-zip');
const url='https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/fuze-rust-preview-cd1146794103.zip';
const response=await fetch(url,{signal:AbortSignal.timeout(120000)});
if(!response.ok)throw Error('Public data seed HTTP '+response.status);
const bytes=Buffer.from(await response.arrayBuffer());
if(bytes.length!==72496132||createHash('sha256').update(bytes).digest('hex')!=='b64daab564acea4095a528fbb60c9b06cc6e706afbd88a587245a2d36fce38cd')throw Error('Public seed checksum mismatch');
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
