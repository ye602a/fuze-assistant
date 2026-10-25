import {readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {createRequire} from 'node:module';
import path from 'node:path';
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
for(const number of [142]){const pr=await request(`${api}/pulls/${number}`);if(!pr.merged_at)throw Error(`PR ${number} not merged`);}
const master=await request(`${api}/git/ref/heads/master`);if(master.object.sha!==source)throw Error('Master moved since build');
const name=`fuze-rust-preview-${revision}.zip`,zipBytes=await readFile(path.join(directory,name)),zip=new AdmZip(zipBytes),build=JSON.parse(zip.readAsText('build.json'));
if(build.revision!==revision||build.version!=='0.7.2'||build.uiFrontend!=='slint')throw Error('Wrong ZIP identity');
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
const setupName=`FUZE-Setup-0.7.2-preview-${revision}-windows-x64.exe`,setup=await readFile(path.join(directory,setupName));
const evidence={source,revision,version:'0.7.2',sequence:manifest.sequence,zipSha256:hash(zipBytes),setupSha256:hash(setup),signed:false,buildRun:Number(process.env.GITHUB_RUN_ID),buildEvidence:receipt,appLocalRuntime:crt,updateValidation:validation};
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
const body=`## FUZE v0.7.2 测试版

- [下载安装包（Setup.exe）](${base+setupName})
- [下载便携 ZIP](${base+name})

Setup 未签名，所需 VC++ DLL 已随程序提供。可选择原目录覆盖安装；便携包请完整解压后运行 \`Start-preview.cmd\`。

### 本次更新

- 选人界面补齐本次二选一、三选一卡片，与公共候选池分开显示；接口暂无数据时明确提示。
- 修复多个空槽同时显示“当前”：本人未选英雄时显示“本人”，选定后显示“当前”。
- 减少空闲性能日志，保留连接、选人、等级变化和识别错误等对局过程。
- 持续识别失败或卡片未读全时，每 2 秒保存本地截图；正常状态每 30 秒保存一条。上限提高至 1200 条 / 256 MiB，满后停止并提示，不自动上传。

遇到识别问题后，在「偏好设置 → 帮助与反馈」打开截图目录，检查相关 PNG 和同名 JSON 后再提供反馈。

版本 **0.7.2** · 构建：\`${revision}\` · [对应源码](https://github.com/ye602a/fuze-assistant/commit/${source})

[PR #142](https://github.com/ye602a/fuze-assistant/pull/142) 已通过 CI 后合并。ZIP、在线更新包和 Setup 来自同一提交、同一载荷，包体与更新检查在 GitHub Windows 环境完成。

[构建记录](${base}build-evidence-${revision}.json) · [构建工作流](https://github.com/ye602a/fuze-assistant/actions/runs/${process.env.GITHUB_RUN_ID})
`;
await request(`${api}/releases/${preview.id}`,'PATCH',{name:'FUZE v0.7.2 测试版',body,prerelease:true,make_latest:'false'});
console.log(JSON.stringify({published:true,source,revision,version:'0.7.2',previous:previous.revision,zip:name,setup:setupName}));
