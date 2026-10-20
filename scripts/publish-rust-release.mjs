// Explicit master-only publication of immutable, successful stage artifacts. Never builds or runs FUZE.
import {readFile, writeFile} from 'node:fs/promises'
import {createHash} from 'node:crypto'
const api = 'https://api.github.com/repos/ye602a/fuze-assistant'
const version = process.env.FUZE_RELEASE_VERSION, runId = process.env.FUZE_BUILD_RUN
if (!/^\d+\.\d+\.\d+$/.test(version || '') || !/^\d+$/.test(runId || '') ||
    process.env.GITHUB_REF !== 'refs/heads/master' || process.env.GITHUB_REPOSITORY !== 'ye602a/fuze-assistant')
 throw Error('publication-input-or-context')
const hash = (bytes, algorithm='sha256') => createHash(algorithm).update(bytes).digest('hex')
const headers = {Authorization:`Bearer ${process.env.GH_TOKEN}`,Accept:'application/vnd.github+json',
 'X-GitHub-Api-Version':'2022-11-28','User-Agent':'FUZE-verified-publisher'}
async function request(url, method='GET', body, raw=false, allowMissing=false) {
 const response = await fetch(url, {method,headers:{...headers,...(body?{'Content-Type':raw?'application/octet-stream':'application/json'}:{})},
  body:body?(raw?body:JSON.stringify(body)):undefined,signal:AbortSignal.timeout(180000)})
 if (response.status === 404 && allowMissing) return null
 if (!response.ok) throw Error(`publication-http-${response.status}`)
 return response.json()
}
const json = async file => JSON.parse((await readFile(file,'utf8')).replace(/^\uFEFF/,''))
const run = await request(`${api}/actions/runs/${runId}`)
if (run.conclusion !== 'success' || run.event !== 'workflow_dispatch' ||
    run.path !== '.github/workflows/rust-prototypes.yml' || run.head_repository?.full_name !== 'ye602a/fuze-assistant')
 throw Error('publication-unverified-run')
const source = run.head_sha, revision = source.slice(0,12), tag = `rust-v${version}`
const ancestry = await request(`${api}/compare/${source}...master`)
if (!['ahead','identical'].includes(ancestry.status)) throw Error('publication-source-not-integrated')
const receipt = await json('release-payload/installer-build.json')
const validation = await json('release-payload/installer-validation.json')
const lifecycle = await json('stage-evidence/update-validation.json')
if (receipt.version !== version || receipt.installerSource !== source || receipt.packageRevision !== revision ||
    receipt.signed !== false || receipt.signatureStatus !== 'NotSigned' || validation.revision !== revision || validation.gameAccess !== false ||
    !['setupExe','explicitPolicyRespected','installed','repeatInstall','uninstalled','userDataPreserved'].every(k=>validation[k]===true) ||
    lifecycle.game_access !== false ||
    !['local_setup','setup_idempotent','uninstall','installed','repeat_launch','health_failure_rollback','process_start_failure_rollback'].every(k=>lifecycle[k]===true))
 throw Error('publication-receipt-or-validation')
const zipName = `fuze-rust-preview-${revision}.zip`, setupName = `FUZE-Setup-${version}-windows-x64.exe`
const zip = await readFile(`release-payload/${zipName}`), setup = await readFile(`release-payload/${setupName}`)
const zipHash = hash(zip), setupHash = hash(setup)
if (!(await readFile(`release-payload/${zipName}.sha256`,'utf8')).startsWith(zipHash) ||
    receipt.installerSha256 !== setupHash ||
    !(await readFile(`release-payload/${setupName}.sha256`,'utf8')).startsWith(setupHash))
 throw Error('publication-package-digest')
const manifestBytes = await readFile('release-payload/rust-update.json'), manifest = JSON.parse(manifestBytes)
if (manifest.revision !== revision || manifest.channel !== 'rust-preview' ||
    manifest.archive !== `Fuze-Rust-${revision}.fuz` || !Number.isSafeInteger(manifest.sequence)) throw Error('publication-manifest')
const fuz = await readFile(`release-payload/${manifest.archive}`), map = await readFile(`release-payload/${manifest.archive}.blockmap`)
if (hash(fuz,'sha512') !== manifest.sha512 || fuz.length !== manifest.size || hash(map) !== manifest.blockmap_sha256)
 throw Error('publication-update-digest')
async function upload(release, name, bytes) {
 const asset = release.assets.find(a=>a.name===name) || await request(
  `https://uploads.github.com/repos/ye602a/fuze-assistant/releases/${release.id}/assets?name=${encodeURIComponent(name)}`, 'POST', bytes, true)
 if (asset.size !== bytes.length || asset.digest !== `sha256:${hash(bytes)}`) throw Error('publication-asset-collision')
 console.log(`Verified asset ${name}`)
}
// Never repoint an existing release tag or replace assets under the same filename.
let ref = await request(`${api}/git/ref/tags/${tag}`,'GET',undefined,false,true)
if (!ref) {
 const object = await request(`${api}/git/tags`,'POST',{tag,message:`FUZE v${version}`,object:source,type:'commit'})
 ref = await request(`${api}/git/refs`,'POST',{ref:`refs/tags/${tag}`,sha:object.sha})
}
const tagged = ref.object.type === 'tag' ? (await request(`${api}/git/tags/${ref.object.sha}`)).object.sha : ref.object.sha
if (tagged !== source) throw Error('publication-tag-collision')
const stableZip = `FUZE-v${version}-windows-x64.zip`
const download = `https://github.com/ye602a/fuze-assistant/releases/download/${tag}`
const notes = `## FUZE v${version}\n\n`+
 `**[下载 Windows x64 安装器（未签名）](${download}/${setupName})** · [便携 ZIP](${download}/${stableZip})\n\n`+
 `安装器先检查本构建要求的 VC++ x64 运行库，再复用原生安装、开始菜单、应用登记和卸载逻辑。缺库时由你确认微软下载和安装；不自动启动 FUZE、不自动重启 Windows。卸载保留用户数据。现有 Fuze Rust Preview 名称与数据目录保留兼容。\n\n`+
 `最低 VC++ 支持基线：${receipt.minimumVcRuntime}。源码：[${revision}](https://github.com/ye602a/fuze-assistant/commit/${source})。Setup、ZIP 和更新频道为同一程序构建。\n\n`+
 `已通过[隔离阶段验收](https://github.com/ye602a/fuze-assistant/actions/runs/${runId})：真实 Setup 安装、重复安装、卸载与数据保留，以及原生更新健康失败/启动失败回滚与再次启动。未签名；这些检查不代替玩家电脑、IME、混合 DPI 和实际游戏验收。\n\n`+
 `当前 Rust 版未接入向数据站上传对局，赛后战绩保存在本地；GitHub 诊断反馈是独立可选功能。数据读取接口不证明站点归属。\n\n`+
 `ZIP 完整解压后使用 Start-this-build.cmd；也可使用 Install-local.cmd。公开更新无需令牌。遇到问题导出脱敏反馈并注明构建 ${revision}，不要上传原始日志或账号信息。\n`
let stable = await request(`${api}/releases/tags/${tag}`,'GET',undefined,false,true)
if (!stable) stable = await request(`${api}/releases`,'POST',{tag_name:tag,name:`FUZE v${version}`,body:notes,draft:true,prerelease:false,make_latest:'false'})
for (const [name,bytes] of [[setupName,setup],[stableZip,zip]]) {
 await upload(stable,name,bytes)
 await upload(stable,name+'.sha256',Buffer.from(`${hash(bytes)}  ${name}\n`))
}
for (const name of ['installer-build.json','installer-validation.json']) await upload(stable,name,await readFile(`release-payload/${name}`))
const preview = await request(`${api}/releases/tags/rust-preview`)
if (!preview.prerelease || preview.draft) throw Error('publication-update-channel')
for (const [name,bytes] of [[zipName,zip],[zipName+'.sha256',Buffer.from(`${zipHash}  ${zipName}\n`)],[manifest.archive,fuz],[manifest.archive+'.blockmap',map]])
 await upload(preview,name,bytes)
const old = preview.assets.find(a=>a.name==='rust-update.json')
if (!old) throw Error('publication-current-pointer-missing')
const response = await fetch(old.browser_download_url,{signal:AbortSignal.timeout(60000)})
if (!response.ok) throw Error('publication-current-pointer-download')
const previous = await response.json()
if (previous.revision !== revision) {
 if (previous.sequence >= manifest.sequence || preview.assets.some(a=>a.name===`rust-update-before-${revision}.json`)) throw Error('publication-stale-sequence')
 await request(`${api}/releases/assets/${old.id}`,'PATCH',{name:`rust-update-before-${revision}.json`})
 try { await upload({...preview,assets:preview.assets.filter(a=>a.id!==old.id)},'rust-update.json',manifestBytes) }
 catch (error) {
  // A timed-out upload may have succeeded. Reconcile before restoring the prior pointer.
  const fresh = await request(`${api}/releases/tags/rust-preview`)
  const current = fresh.assets.find(a=>a.name==='rust-update.json')
  if (!current) await request(`${api}/releases/assets/${old.id}`,'PATCH',{name:'rust-update.json'})
  else if (current.digest === `sha256:${hash(manifestBytes)}`) console.log('Reconciled published pointer')
  else throw Error('publication-pointer-needs-review')
  if (!current) throw error
 }
} else if (previous.sha512 !== manifest.sha512 || previous.sequence !== manifest.sequence) throw Error('publication-existing-pointer-mismatch')
await request(`${api}/releases/${preview.id}`,'PATCH',{name:`FUZE Rust 更新频道 ${revision}`,body:notes,prerelease:true,make_latest:'false'})
await request(`${api}/releases/${stable.id}`,'PATCH',{body:notes,draft:false,prerelease:false,make_latest:'true'})
await writeFile('publication-evidence.json',JSON.stringify({source,revision,version,runId,tag,setupSha256:setupHash,zipSha256:zipHash,sequence:manifest.sequence,published:true},null,2))
console.log(`Published ${tag}`)
