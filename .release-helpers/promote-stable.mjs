// Publish the already verified binaries without rebuilding or changing the legacy update endpoint.
import {createHash} from 'node:crypto';
const api='https://api.github.com/repos/ye602a/fuze-assistant';
const source=process.env.SOURCE_SHA, revision=source.slice(0,12);
const tag='rust-v0.7.5', version='0.7.5';
const hash=b=>createHash('sha256').update(b).digest('hex');
async function request(url,method='GET',body,raw=false,missing=false){
 const r=await fetch(url,{method,headers:{Authorization:`Bearer ${process.env.GH_TOKEN}`,Accept:'application/vnd.github+json','User-Agent':'FUZE-release-promotion',...(body?{'Content-Type':raw?'application/octet-stream':'application/json'}:{})},body:body?(raw?body:JSON.stringify(body)):undefined,signal:AbortSignal.timeout(180000)});
 if(missing&&r.status===404)return null;
 if(!r.ok)throw Error(`GitHub ${r.status}`);
 return r.status===204 ? null : r.json();
}
const preview=await request(`${api}/releases/tags/rust-preview`);
if(preview.draft||!preview.prerelease)throw Error('Unexpected update endpoint state');
async function download(name){
 const asset=preview.assets.find(a=>a.name===name);
 if(!asset)throw Error('Missing '+name);
 const r=await fetch(asset.browser_download_url,{signal:AbortSignal.timeout(180000)});
 if(!r.ok)throw Error('Download HTTP '+r.status);
 const bytes=Buffer.from(await r.arrayBuffer());
 if(bytes.length!==asset.size||asset.digest!==`sha256:${hash(bytes)}`)throw Error('Digest mismatch '+name);
 return bytes;
}
const [receiptBytes,zip,setup]=await Promise.all([
 download(`build-evidence-${revision}.json`),
 download(`fuze-rust-preview-${revision}.zip`),
 download(`FUZE-Setup-${version}-preview-${revision}-windows-x64.exe`),
]);
const receipt=JSON.parse(receiptBytes);
if(receipt.source!==source||receipt.version!==version||!receipt.buildEvidence.sourceClean||!receipt.updateValidation.package_verified||hash(zip)!==receipt.zipSha256||hash(setup)!==receipt.setupSha256)throw Error('Build receipt mismatch');
const zipName=`FUZE-${version}-windows-x64.zip`,setupName=`FUZE-Setup-${version}-windows-x64.exe`;
const base=`https://github.com/ye602a/fuze-assistant/releases/download/${tag}/`;
let body=preview.body.replace('## FUZE v0.7.5 测试版','## FUZE v0.7.5');
body=body.replaceAll(`https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/fuze-rust-preview-${revision}.zip`,base+zipName)
 .replaceAll(`https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/FUZE-Setup-${version}-preview-${revision}-windows-x64.exe`,base+setupName)
 .replaceAll(`https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/build-evidence-${revision}.json`,base+`build-evidence-${revision}.json`);
let release=await request(`${api}/releases/tags/${tag}`,'GET',undefined,false,true);
if(!release)release=await request(`${api}/releases`,'POST',{tag_name:tag,target_commitish:source,name:'FUZE v0.7.5',body,draft:true,prerelease:false});
if(release.target_commitish!==source)throw Error('Unexpected existing release source');
for(const [name,bytes] of [[zipName,zip],[setupName,setup],[zipName+'.sha256',Buffer.from(`${hash(zip)}  ${zipName}\n`)],[setupName+'.sha256',Buffer.from(`${hash(setup)}  ${setupName}\n`)],[`build-evidence-${revision}.json`,receiptBytes]]){
 let asset=release.assets.find(a=>a.name===name);
 if(asset?.digest!==`sha256:${hash(bytes)}`){
  const uploaded=await request(`https://uploads.github.com/repos/ye602a/fuze-assistant/releases/${release.id}/assets?name=${encodeURIComponent(name+'.pending-'+revision)}`,'POST',bytes,true);
  if(uploaded.size!==bytes.length||uploaded.digest!==`sha256:${hash(bytes)}`)throw Error('Staged asset mismatch '+name);
  if(asset)await request(`${api}/releases/assets/${asset.id}`,'DELETE');
  asset=await request(`${api}/releases/assets/${uploaded.id}`,'PATCH',{name});
 }
 if(asset.size!==bytes.length||asset.digest!==`sha256:${hash(bytes)}`)throw Error('Published asset mismatch '+name);
 console.log('Verified '+name);
}

await request(`${api}/releases/${release.id}`,'PATCH',{target_commitish:source,name:'FUZE v0.7.5',body,draft:false,prerelease:false,make_latest:'true'});
const latest=await request(`${api}/releases/latest`);
const kept=await request(`${api}/releases/tags/rust-preview`);
if(latest.id!==release.id||latest.draft||latest.prerelease||!kept.prerelease||JSON.stringify(kept.assets.map(a=>[a.id,a.digest]))!==JSON.stringify(preview.assets.map(a=>[a.id,a.digest])))throw Error('Final release or update endpoint verification failed');
console.log(JSON.stringify({published:true,tag,version,source,url:latest.html_url,latest:true,updateEndpointPreserved:true}));
