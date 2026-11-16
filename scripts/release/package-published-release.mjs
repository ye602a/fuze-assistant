import {readFile,writeFile,mkdir} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import AdmZip from 'adm-zip';
import {unpackPublishedUpdate} from './unpack-published-update.mjs';
import {verifyBinaries} from './verify-binaries.mjs';
import {releaseBody} from './release-body.mjs';
const api='https://api.github.com/repos/ye602a/fuze-assistant';
const version=process.env.FUZE_RELEASE_VERSION;
if(!/^0\.7\.\d+$/.test(version||''))throw Error('Invalid version');
const out=path.resolve('rust/artifacts/repackage'),tag=`rust-v${version}`;
const hash=(bytes,algorithm='sha256')=>createHash(algorithm).update(bytes).digest('hex');
async function request(url,method='GET',body,raw=false){
 const r=await fetch(url,{method,headers:{Authorization:`Bearer ${process.env.GH_TOKEN}`,Accept:'application/vnd.github+json',...(body?{'Content-Type':raw?'application/octet-stream':'application/json'}:{})},body:body?(raw?body:JSON.stringify(body)):undefined,signal:AbortSignal.timeout(180000)});
 if(!r.ok)throw Error(`GitHub ${r.status}`);return r.json();
}
const preview=await request(`${api}/releases/tags/rust-preview`);
const snapshot=assets=>assets.map(a=>[a.id,a.name,a.digest,a.size]).sort((a,b)=>a[0]-b[0]);
async function download(name){
 const a=preview.assets.find(a=>a.name===name);if(!a)throw Error('Missing '+name);
 const r=await fetch(a.browser_download_url,{signal:AbortSignal.timeout(180000)});if(!r.ok)throw Error('Download failed');
 const b=Buffer.from(await r.arrayBuffer());if(b.length!==a.size||`sha256:${hash(b)}`!==a.digest)throw Error('Asset digest mismatch');return b;
}
if(process.argv[2]==='prepare'){
 const description=JSON.parse(await download('rust-update-v2.json')),m=description.manifest;
 if(description.version!==version||!/^[a-f0-9]{12}$/.test(m.revision))throw Error('Online version mismatch');
 const receipt=JSON.parse(await download(`build-evidence-${m.revision}.json`));
 if(receipt.version!==version||receipt.revision!==m.revision||!receipt.source.startsWith(m.revision)||!receipt.buildEvidence.completed||!receipt.buildEvidence.sourceClean||!receipt.updateValidation.package_verified)throw Error('Unverified build');
 const fuz=await download(m.archive),blockmap=await download(m.archive+'.blockmap');
 if(fuz.length!==m.size||hash(fuz,'sha512')!==m.sha512||hash(blockmap)!==m.blockmap_sha256)throw Error('Update integrity mismatch');
 const entries=unpackPublishedUpdate(fuz,m.revision),zip=new AdmZip();
 for(const {name,data}of entries)zip.addFile(name,data);
 const build=JSON.parse(zip.readAsText('build.json'));
 if(build.version!==version||build.revision!==m.revision||build.desktopExecutable!=='FUZE.exe')throw Error('Wrong player identity');
 verifyBinaries(zip,build,receipt.buildEvidence.binaries);
 const crt=JSON.parse(zip.readAsText('vc-runtime-files.json'));
 for(const [name,digest]of Object.entries(crt))if(!zip.readFile(name)||hash(zip.readFile(name))!==digest)throw Error('Runtime mismatch');
 for(const name of ['FUZE.exe','fuze-update.exe','README.txt','resources/inference.onnx','resources/dictionary.json'])if(!zip.readFile(name))throw Error('Incomplete player bundle: '+name);
 await mkdir(out,{recursive:true});
 for(const {name,data}of entries){const dest=path.join(out,'payload',name);await mkdir(path.dirname(dest),{recursive:true});await writeFile(dest,data);}
 zip.writeZip(path.join(out,`FUZE-${version}-windows-x64.zip`));
 const installer=await fetch(`https://raw.githubusercontent.com/ye602a/fuze-assistant/${receipt.source}/installer/rust-setup.iss`);
 if(!installer.ok)throw Error('Missing installer source');await writeFile(path.join(out,'rust-setup.iss'),await installer.text());
 await writeFile(path.join(out,'context.json'),JSON.stringify({receipt,description,previewAssets:snapshot(preview.assets)}));
 console.log(JSON.stringify({prepared:true,version,source:receipt.source,files:entries.length,binariesVerified:true}));
}else if(process.argv[2]==='publish'){
 const {receipt,description,previewAssets}=JSON.parse(await readFile(path.join(out,'context.json')));
 if(JSON.stringify(snapshot(preview.assets))!==JSON.stringify(previewAssets))throw Error('Online assets changed');
 const release=await request(`${api}/releases/tags/${tag}`);
 const ref=await request(`${api}/git/ref/tags/${tag}`);
 if(ref.object.type!=='commit'||ref.object.sha!==receipt.source||release.draft||release.prerelease)throw Error('Release source mismatch');
 const files=[];
 for(const name of [`FUZE-${version}-windows-x64.zip`,`FUZE-Setup-${version}-windows-x64.exe`]){
  const b=await readFile(path.join(out,name));if(!b.length)throw Error('Empty package');
  files.push([name,b],[name+'.sha256',Buffer.from(`${hash(b)}  ${name}\n`)]);
 }
 const evidence={version,source:receipt.source,revision:receipt.revision,originalBuildRun:receipt.buildRun,packagingRun:Number(process.env.GITHUB_RUN_ID),method:'Repackage verified published update; no recompilation',files:Object.fromEntries(files.filter(([n])=>!n.endsWith('.sha256')).map(([n,b])=>[n,{size:b.length,sha256:hash(b)}]))};
 files.push([`packaging-evidence-${receipt.revision}.json`,Buffer.from(JSON.stringify(evidence,null,2))]);
 for(const [name,b]of files){
  let a=release.assets.find(a=>a.name===name);
  if(!a)a=await request(`https://uploads.github.com/repos/ye602a/fuze-assistant/releases/${release.id}/assets?name=${encodeURIComponent(name)}`,'POST',b,true);
  if(a.size!==b.length||a.digest!==`sha256:${hash(b)}`)throw Error('Published asset mismatch: '+name);
 }
 const notes=description.notes.split(/\r?\n/).filter(line=>!line.includes('本次仅提供')&&!line.includes('不提供新的 Setup')).join('\n').trim();
 const body=releaseBody({version,source:receipt.source,notes,buildRun:receipt.buildRun,buildSetup:true})
  .replace(`[构建记录](https://github.com/ye602a/fuze-assistant/releases/download/${tag}/build-evidence-${receipt.revision}.json)`,`[原始构建记录](https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/build-evidence-${receipt.revision}.json)`)
  +`\n[完整下载打包记录](https://github.com/ye602a/fuze-assistant/releases/download/${tag}/packaging-evidence-${receipt.revision}.json) · [打包工作流](https://github.com/ye602a/fuze-assistant/actions/runs/${process.env.GITHUB_RUN_ID})\n`;
 await request(`${api}/releases/${release.id}`,'PATCH',{name:`FUZE v${version}`,body,make_latest:'true'});
 const latest=await request(`${api}/releases/latest`),kept=await request(`${api}/releases/tags/rust-preview`);
 if(latest.id!==release.id||JSON.stringify(snapshot(kept.assets))!==JSON.stringify(previewAssets))throw Error('Publication verification failed');
 console.log(JSON.stringify({published:true,url:latest.html_url,...evidence}));
}else throw Error('Expected prepare or publish');
