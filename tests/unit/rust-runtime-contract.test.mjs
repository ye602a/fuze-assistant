import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync, writeFileSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { makeRuntimeContract, sha256, installCommand } from '../../scripts/rust-runtime-contract.mjs'
import { assessReadiness, inspectPe } from '../../scripts/rust-release-readiness.mjs'

// Headers/import data only, deliberately no executable code.
function peFixture() {
  const b = Buffer.alloc(2048)
  b.writeUInt16LE(0x5a4d); b.writeUInt32LE(128, 0x3c); b.writeUInt32LE(0x4550, 128)
  b.writeUInt16LE(0x8664, 132); b.writeUInt16LE(1, 134); b.writeUInt16LE(240, 148)
  b.writeUInt16LE(0x20b, 152); b.writeUInt32LE(16, 260)
  b.writeUInt32LE(0x1000, 272); b.writeUInt32LE(40, 276)
  b.writeUInt32LE(0x1000, 404); b.writeUInt32LE(1536, 408); b.writeUInt32LE(512, 412)
  b.writeUInt32LE(0x1100, 524); b.write('VCRUNTIME140.dll\0', 768)
  return b
}
function fixture() {
  const policy = JSON.parse(readFileSync(new URL('../../rust/vc-runtime-policy.json', import.meta.url)))
  const binaries = {}, evidence = { schema: 1, sourceClean: true, completed: true, revision: '123456789abc', frontend: 'slint', compilerVersion: '19.44.35229.0', linkerVersion: '14.44.35229.0', binaries: {} }
  for (const name of ['fuze-rust-host.exe','fuze-ui-lab.exe','fuze-ocr-worker.exe','fuze-update.exe',...Object.keys(policy.thirdPartySha256)]) {
    binaries[name] = peFixture()
    if (name.endsWith('.exe')) evidence.binaries[name] = sha256(binaries[name])
    else policy.thirdPartySha256[name] = sha256(binaries[name])
  }
  const policyBytes = Buffer.from(JSON.stringify(policy))
  evidence.policySha256 = sha256(policyBytes)
  return { policyBytes, evidence, binaries, revision: evidence.revision, frontend: 'slint', onnxVersion: policy.onnxRuntimeVersion }
}
test('runtime record binds clean build, toolchain, reviewed policy and exact native bytes', () => {
  const f = fixture(), c = makeRuntimeContract(f)
  assert.equal(c.minimumVersion, '14.51.36247.0')
  assert.deepEqual(c.requiredDlls, ['vcruntime140.dll'])
  assert.equal(c.compilerVersion, f.evidence.compilerVersion)
  assert.equal(Object.keys(c.binaries).length, 8)
})
test('missing, stale, incomplete or newer-toolchain receipts never label existing binaries', () => {
  for (const mutate of [f=>f.evidence=null, f=>f.evidence.completed=false, f=>f.evidence.sourceClean=false,
    f=>f.evidence.revision='aaaaaaaaaaaa', f=>f.evidence.frontend='win32', f=>f.evidence.policySha256='b'.repeat(64),
    f=>f.evidence.compilerVersion='19.99.1.0', f=>f.evidence.linkerVersion='14.99.1.0',
    f=>f.evidence.binaries['fuze-update.exe']='b'.repeat(64)]) {
    const f = fixture(); mutate(f); assert.throws(()=>makeRuntimeContract(f), /runtime-/)
  }
})
test('third-party changes, wrong architecture and new CRT imports require review', () => {
  for (const mutate of [f=>f.onnxVersion='9.0.0', f=>f.frontend='other',
    f=>f.binaries['DirectML.dll'][100]=1,
    f=>{f.binaries['fuze-update.exe'].writeUInt16LE(0xaa64,132);f.evidence.binaries['fuze-update.exe']=sha256(f.binaries['fuze-update.exe'])},
    f=>{f.binaries['fuze-update.exe'].write('VCRUNTIME999.dll\0',768);f.evidence.binaries['fuze-update.exe']=sha256(f.binaries['fuze-update.exe'])}]) {
    const f = fixture(); mutate(f); assert.throws(()=>makeRuntimeContract(f), /runtime-/)
  }
})
test('readiness consumes recorded minimum and refuses weakened overrides or mismatched bytes', () => {
  const f = fixture(), vcRuntime = makeRuntimeContract(f)
  const build = {revision:f.revision,sequence:123,uiFrontend:'slint',runtime:f.onnxVersion,vcRuntime}
  const binaries=Object.entries(f.binaries).map(([name,bytes])=>({name,sha256:sha256(bytes),...inspectPe(bytes)}))
  const windows={x64Process:true,binaries:binaries.map(b=>({...b,signatureStatus:'Valid',signerSha256:'a'.repeat(64),timestampPresent:true})),runtime:[{name:'vcruntime140.dll',present:true,architecture:'x64',version:vcRuntime.minimumVersion}]}
  const policy={publisherSha256:'a'.repeat(64)}
  const result=assessReadiness(build,binaries,windows,policy)
  assert.equal(result.minimumVcRuntimeSource,'build-record'); assert.equal(result.staticChecksPassed,true)
  assert.equal(result.releaseReady,false)
  assert.throws(()=>assessReadiness(build,binaries,windows,{...policy,minimumVcRuntime:'14.40.1.0'}),/policy-weakened/)
  build.vcRuntime.binaries['fuze-update.exe']='c'.repeat(64)
  assert.equal(assessReadiness(build,binaries,windows,policy).staticChecksPassed,false)
  build.vcRuntime.requiredDlls=['msvcp140.dll']
  assert.ok(assessReadiness(build,binaries,windows,policy).blockers.includes('runtime-record-dependencies'))
})
test('CMD enters bootstrap first and cannot fall through to native setup if helper is absent', () => {
  const cmd=installCommand()
  assert.ok(cmd.indexOf('exit /b 2')<cmd.indexOf('powershell.exe'))
  assert.ok(cmd.includes('Install-local.ps1'))
  assert.doesNotMatch(cmd,/fuze-update|ExecutionPolicy|Set-ExecutionPolicy/)
})
test('Windows bootstrap exercises failures and confirmation using effects that cannot launch programs', {skip:process.platform!=='win32'}, () => {
  const root=mkdtempSync(path.join(tmpdir(),'fuze-runtime-contract-'))
  const env={...process.env}; delete env.PSModulePath
  const script=fileURLToPath(new URL('./rust-runtime-bootstrap.ps1',import.meta.url))
  const result=spawnSync(path.join(process.env.SystemRoot,'System32/WindowsPowerShell/v1.0/powershell.exe'),['-NoProfile','-NonInteractive','-File',script,'-FixtureRoot',path.join(root,'payload with spaces')],{env,encoding:'utf8',windowsHide:true,timeout:30000})
  assert.equal(result.status,0,result.stderr)
  const evidence=JSON.parse(result.stdout)
  assert.equal(evidence.checks,34);assert.equal(evidence.appLaunched,false);assert.equal(evidence.runtimeInstalled,false)
  const payload=path.join(root,'payload with spaces')
  const check=(extra=[])=>spawnSync(path.join(process.env.SystemRoot,'System32/WindowsPowerShell/v1.0/powershell.exe'),['-NoProfile','-NonInteractive','-File',path.join(payload,'Install-local.ps1'),'-CheckOnly',...extra],{env,encoding:'utf8',windowsHide:true,timeout:30000})
  const entry=check()
  assert.ok([0,2].includes(entry.status),entry.stderr)
  assert.ok(['ready','runtime-required'].includes(JSON.parse(entry.stdout).state))
  const portable=check(['-LaunchThisBuild'])
  assert.equal(portable.status,entry.status,portable.stderr)
  assert.equal(JSON.parse(portable.stdout).state,JSON.parse(entry.stdout).state)
  writeFileSync(path.join(payload,'build.json'),JSON.stringify({revision:'123456789abc',uiFrontend:'slint',runtime:'1.26.0'}))
  const legacy=check()
  assert.equal(legacy.status,2,legacy.stderr);assert.equal(JSON.parse(legacy.stdout).state,'unknown-build')
})
