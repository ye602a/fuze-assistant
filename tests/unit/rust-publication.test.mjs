import {fileURLToPath} from 'node:url'
import test from 'node:test'
import assert from 'node:assert/strict'
import {spawnSync} from 'node:child_process'
import {mkdtemp, mkdir, writeFile} from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
const script=fileURLToPath(new URL('../../scripts/publish-rust-release.mjs',import.meta.url))
for (const scenario of ['input','run','ancestry','receipt','digest']) {
 test(`publisher rejects ${scenario} without any remote write`, async () => {
  const dir=await mkdtemp(path.join(os.tmpdir(),'fuze-publish-fixture-'))
  const source='a'.repeat(40), revision=source.slice(0,12)
  await mkdir(path.join(dir,'release-payload'))
  await mkdir(path.join(dir,'stage-evidence'))
  const receipt={version:'0.7.1',installerSource:source,packageRevision:revision,signed:false,signatureStatus:"NotSigned",installerSha256:'incorrect'}
  const validation={revision,gameAccess:false,setupExe:true,explicitPolicyRespected:true,installed:scenario!=='receipt',repeatInstall:true,uninstalled:true,userDataPreserved:true}
  const lifecycle={game_access:false,local_setup:true,setup_idempotent:true,uninstall:true,installed:true,repeat_launch:true,health_failure_rollback:true,process_start_failure_rollback:true}
  for(const [name,value] of [['release-payload/installer-build.json',receipt],['release-payload/installer-validation.json',validation],['stage-evidence/update-validation.json',lifecycle]])
   await writeFile(path.join(dir,name),JSON.stringify(value))
  for(const name of [`fuze-rust-preview-${revision}.zip`,`fuze-rust-preview-${revision}.zip.sha256`,'FUZE-Setup-0.7.1-windows-x64.exe'])
   await writeFile(path.join(dir,'release-payload',name),'synthetic non-executable fixture')
  const mock=`
   globalThis.fetch=async (url,options={})=>{
    if(options.method && options.method!=='GET') throw Error('UNEXPECTED_REMOTE_WRITE');
    let data;
    if(url.includes('/actions/runs/')) data=${JSON.stringify({conclusion:scenario==='run'?'failure':'success',event:'workflow_dispatch',path:'.github/workflows/rust-prototypes.yml',head_repository:{full_name:'ye602a/fuze-assistant'},head_sha:source})};
    else if(url.includes('/compare/')) data={status:${JSON.stringify(scenario==='ancestry'?'diverged':'ahead')}};
    else throw Error('UNEXPECTED_REMOTE_READ');
    return new Response(JSON.stringify(data),{status:200});
   };`
  const result=spawnSync(process.execPath,['--import','data:text/javascript,'+encodeURIComponent(mock),script],{
   cwd:dir,encoding:'utf8',windowsHide:true,timeout:10000,
   env:{...process.env,GH_TOKEN:'synthetic-test',FUZE_RELEASE_VERSION:scenario==='input'?'bad':'0.7.1',FUZE_BUILD_RUN:'123',GITHUB_REF:'refs/heads/master',GITHUB_REPOSITORY:'ye602a/fuze-assistant'}
  })
  assert.notEqual(result.status,0)
  assert.doesNotMatch(result.stderr,/UNEXPECTED_REMOTE/)
  const expected={input:'publication-input-or-context',run:'publication-unverified-run',ancestry:'publication-source-not-integrated',receipt:'publication-receipt-or-validation',digest:'publication-package-digest'}[scenario]
  assert.ok(result.stderr.includes(expected),result.stderr)
 })
}
