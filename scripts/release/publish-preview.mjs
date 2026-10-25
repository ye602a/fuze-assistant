import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import path from 'node:path';
const version=process.env.FUZE_RELEASE_VERSION;
if(!/^0\.7\.\d+$/.test(version||''))throw Error('Release version required');
const source=process.env.SOURCE_SHA,revision=source?.slice(0,12),root=process.cwd();
if(!/^[a-f0-9]{40}$/.test(source||''))throw Error('Source SHA required');
const require=createRequire(path.join(root,'package.json')),AdmZip=require('adm-zip');
const directory=path.join(root,'rust/artifacts/preview'),api='https://api.github.com/repos/ye602a/fuze-assistant';
const hash=(b,a='sha256')=>createHash(a).update(b).digest('hex');
async function request(url,method='GET',body,raw=false){
 const response=await fetch(url,{method,headers:{Authorization:`Bearer ${process.env.GH_TOKEN}`,Accept:'application/vnd.github+json','User-Agent':'FUZE-preview-publisher',...(body?{'Content-Type':raw?'application/octet-stream':'application/json'}:{})},body:body?(raw?body:JSON.stringify(body)):undefined,signal:AbortSignal.timeout(180000)});
 if(!response.ok)throw Error(`GitHub ${response.status}: ${await response.text()}`);
 return response.json();
}
const master=await request(`${api}/git/ref/heads/master`);if(master.object.sha!==source)throw Error('Master moved since build');
const name=`fuze-rust-preview-${revision}.zip`,zipBytes=await readFile(path.join(directory,name)),zip=new AdmZip(zipBytes),build=JSON.parse(zip.readAsText('build.json'));
if(build.revision!==revision||build.version!==version||build.uiFrontend!=='slint')throw Error('Wrong ZIP identity');
const receipt=JSON.parse(await readFile('rust/target/release/fuze-build-evidence.json','utf8'));
if(!receipt.completed||!receipt.sourceClean||receipt.revision!==revision)throw Error('Missing clean build receipt');
for(const [file,digest]of Object.entries(receipt.binaries))if(hash(zip.readFile(file))!==digest)throw Error(`Build binary mismatch: ${file}`);
const crt=JSON.parse(zip.readAsText('vc-runtime-files.json'));for(const [file,digest]of Object.entries(crt))if(hash(zip.readFile(file))!==digest)throw Error('CRT mismatch');
const manifestBytes=await readFile(path.join(directory,'rust-update.json')),manifest=JSON.parse(manifestBytes),fuz=await readFile(path.join(directory,manifest.archive)),map=await readFile(path.join(directory,manifest.archive+'.blockmap'));
if(manifest.revision!==revision||manifest.sequence!==build.sequence||manifest.size!==fuz.length||manifest.sha512!==hash(fuz,'sha512')||manifest.blockmap_sha256!==hash(map))throw Error('Update integrity mismatch');
const validation=JSON.parse(await readFile('rust/artifacts/update-validation.json','utf8'));if(!validation.in_place_replacement||!validation.in_place_hidden_launch||!validation.installed||!validation.health_failure_rollback||!validation.repeat_launch||!validation.uninstall)throw Error('Package lifecycle validation absent');
const preview=await request(`${api}/releases/tags/rust-preview`);if(!preview.prerelease||preview.draft)throw Error('Not a public prerelease');
async function upload(name,bytes){
 const existing=preview.assets.find(a=>a.name===name);
 const asset=existing||await request(`https://uploads.github.com/repos/ye602a/fuze-assistant/releases/${preview.id}/assets?name=${encodeURIComponent(name)}`,'POST',bytes,true);
 if(asset.size!==bytes.length||asset.digest!==`sha256:${hash(bytes)}`)throw Error(`Asset collision or integrity: ${name}`);
 console.log(`Uploaded and verified: ${name}`);
}
const setupName=`FUZE-Setup-${version}-preview-${revision}-windows-x64.exe`,setup=await readFile(path.join(directory,setupName));
const evidence={source,revision,version:version,sequence:manifest.sequence,zipSha256:hash(zipBytes),setupSha256:hash(setup),signed:false,buildRun:Number(process.env.GITHUB_RUN_ID),buildEvidence:receipt,appLocalRuntime:crt,updateValidation:validation};
await writeFile(path.join(directory,`build-evidence-${revision}.json`),JSON.stringify(evidence,null,2));
for(const [file,bytes]of [[name,zipBytes],[name+'.sha256',Buffer.from(`${hash(zipBytes)}  ${name}\n`)],[manifest.archive,fuz],[manifest.archive+'.blockmap',map],[setupName,setup],[setupName+'.sha256',Buffer.from(`${hash(setup)}  ${setupName}\n`)],[`build-evidence-${revision}.json`,Buffer.from(JSON.stringify(evidence,null,2))]])await upload(file,bytes);
const old=preview.assets.find(a=>a.name==='rust-update.json');if(!old)throw Error('Missing current update manifest');
const response=await fetch(old.browser_download_url,{signal:AbortSignal.timeout(60000)});if(!response.ok)throw Error('Cannot read current update manifest');
const previous=await response.json();
if(previous.revision!==revision){
 if(previous.sequence>=manifest.sequence)throw Error('Non-monotonic update');
 const backup=`rust-update-before-${revision}.json`;if(preview.assets.some(a=>a.name===backup))throw Error('Manifest backup collision');
 await request(`${api}/releases/assets/${old.id}`,'PATCH',{name:backup});
 preview.assets=preview.assets.filter(a=>a.id!==old.id);
 try{await upload('rust-update.json',manifestBytes);}catch(error){await request(`${api}/releases/assets/${old.id}`,'PATCH',{name:'rust-update.json'});throw error;}
}else if(previous.sha512!==manifest.sha512)throw Error('Published revision has different payload');
const base='https://github.com/ye602a/fuze-assistant/releases/download/rust-preview/';
const body=`## FUZE v${version} 测试版

- [下载安装包（Setup.exe）](${base+setupName})
- [下载便携 ZIP](${base+name})

Setup 未签名，所需 VC++ DLL 已随程序提供。可选择原目录覆盖安装；便携包请完整解压后运行 \`Start-preview.cmd\`。

### 本次更新

${await readFile('docs/RELEASE_NOTES.md','utf8')}

版本 **${version}** · 构建：\`${revision}\` · [对应源码](https://github.com/ye602a/fuze-assistant/commit/${source})

ZIP、在线更新包和 Setup 来自同一提交、同一载荷。

[构建记录](${base}build-evidence-${revision}.json) · [构建工作流](https://github.com/ye602a/fuze-assistant/actions/runs/${process.env.GITHUB_RUN_ID})
`;
await request(`${api}/releases/${preview.id}`,'PATCH',{name:`FUZE v${version} 测试版`,body,prerelease:true,make_latest:'false'});
console.log(JSON.stringify({published:true,source,revision,version:version,previous:previous.revision,zip:name,setup:setupName}));
