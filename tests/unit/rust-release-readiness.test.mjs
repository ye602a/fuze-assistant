import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtemp, mkdir, writeFile, readFile, readdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { nativeFiles, inspectPe, assessReadiness, atLeast } from '../../scripts/rust-release-readiness.mjs'

// Deliberately non-executable bytes: sufficient headers and import tables, no program code.
function peFixture() {
  const b = Buffer.alloc(2048)
  b.writeUInt16LE(0x5a4d); b.writeUInt32LE(128, 0x3c); b.writeUInt32LE(0x4550, 128)
  b.writeUInt16LE(0x8664, 132); b.writeUInt16LE(1, 134); b.writeUInt16LE(240, 148)
  b.writeUInt16LE(0x20b, 152); b.writeUInt32LE(16, 260)
  b.writeUInt32LE(0x1000, 272); b.writeUInt32LE(40, 276)
  b.writeUInt32LE(0x1040, 368); b.writeUInt32LE(64, 372)
  b.writeUInt32LE(0x1000, 404); b.writeUInt32LE(1536, 408); b.writeUInt32LE(512, 412)
  b.writeUInt32LE(0x1100, 524); b.write('VCRUNTIME140.dll\0', 768)
  b.writeUInt32LE(1, 576); b.writeUInt32LE(0x1140, 580); b.write('MSVCP140_ATOMIC_WAIT.dll\0', 832)
  return b
}
const build = { revision: '1c2281940730', sequence: 1792343832, uiFrontend: 'slint', runtime: '1.26.0' }
function evidence() {
  const binaries = nativeFiles.map(name => ({ name, sha256: 'b'.repeat(64), ...inspectPe(peFixture()) }))
  const windows = { x64Process: true,
    binaries: nativeFiles.map(name => ({ name, sha256: 'b'.repeat(64), signatureStatus: 'Valid', signerSha256: 'a'.repeat(64), timestampPresent: true })),
    runtime: ['vcruntime140.dll', 'msvcp140_atomic_wait.dll'].map(name => ({ name, present: true, version: '14.44.35211.0', architecture: 'x64' })),
  }
  return { binaries, windows, policy: { publisherSha256: 'a'.repeat(64), minimumVcRuntime: '14.44.35211.0' } }
}
test('PE inventory includes ordinary and delay imports without loading the image', () => {
  assert.deepEqual(inspectPe(peFixture()), { architecture: 'x64', imports: ['vcruntime140.dll'], delayImports: ['msvcp140_atomic_wait.dll'] })
  const other = peFixture(); other.writeUInt16LE(0xaa64, 132)
  assert.equal(inspectPe(other).architecture, 'unsupported')
})
test('malformed PE addresses, table bounds, paths, and unsupported delay descriptors fail closed', () => {
  for (const mutate of [
    b => b.writeUInt32LE(0xfffffff0, 0x3c), b => b.writeUInt32LE(0xfffffff0, 524),
    b => b.writeUInt32LE(20, 276), b => b.writeUInt32LE(0, 576),
    b => b.write('../private.dll\0', 768), b => b.writeUInt32LE(200000, 408),
    b => b.writeUInt32LE(0, 832), b => b.writeUInt32LE(97, 134),
  ]) {
    const bytes = peFixture(); mutate(bytes)
    assert.throws(() => inspectPe(bytes), /release-pe-invalid/)
  }
  for (const length of [0, 2, 63, 128, 400, 700]) assert.throws(() => inspectPe(peFixture().subarray(0, length)), /release-pe-invalid/)
})
test('ONNX version and presence of runtime DLLs never supply a missing VC minimum or publisher', () => {
  const e = evidence(), report = assessReadiness(build, e.binaries, e.windows)
  assert.equal(report.onnxRuntimeVersion, '1.26.0')
  assert.equal(report.minimumVcRuntime, null)
  assert.equal(report.staticChecksPassed, false)
  assert.ok(report.runtime.every(item => item.meetsDeclaredMinimum === null))
  assert.ok(report.blockers.includes('publisher-identity-unconfigured'))
  assert.ok(report.blockers.includes('vc-runtime-minimum-unrecorded'))
})
test('runtime version comparison uses all four numeric components', () => {
  assert.equal(atLeast('14.9.99999.9', '14.44.1.0'), false)
  assert.equal(atLeast('14.44.35211.0', '14.44.35211.1'), false)
  assert.equal(atLeast('14.44.35211.0', '14.44.35211.0'), true)
  assert.equal(atLeast('14.50.1.0', '14.44.35211.0'), true)
  assert.equal(atLeast('1.26.0', '14.44.35211.0'), false)
})
test('missing, older, wrong-architecture, and unknown imported VC runtime dependencies block', () => {
  for (const [change, reason] of [
    [e => e.windows.runtime.splice(0, 1), 'vc-runtime-missing:vcruntime140.dll'],
    [e => { e.windows.runtime[0].version = '14.40.1.0' }, 'vc-runtime-version:vcruntime140.dll'],
    [e => { e.windows.runtime[0].architecture = 'unsupported' }, 'vc-runtime-architecture:vcruntime140.dll'],
    [e => e.binaries[0].imports.push('msvcp140_future.dll'), 'vc-runtime-missing:msvcp140_future.dll'],
  ]) {
    const e = evidence(); change(e)
    const report = assessReadiness(build, e.binaries, e.windows, e.policy)
    assert.equal(report.staticChecksPassed, false); assert.ok(report.blockers.includes(reason))
  }
})
test('unsigned, untrusted, mismatched publisher, missing timestamp, and changed binaries block', () => {
  for (const [change, reason] of [
    [e => { e.windows.binaries[0].signatureStatus = 'NotSigned' }, 'signature:fuze-rust-host.exe'],
    [e => { e.windows.binaries[4].signatureStatus = 'NotTrusted' }, 'signature:onnxruntime.dll'],
    [e => { e.windows.binaries[0].signerSha256 = 'c'.repeat(64) }, 'publisher:fuze-rust-host.exe'],
    [e => { e.windows.binaries[0].timestampPresent = false }, 'timestamp:fuze-rust-host.exe'],
    [e => { e.windows.binaries[0].sha256 = 'd'.repeat(64) }, 'file-missing-or-changed:fuze-rust-host.exe'],
    [e => e.binaries.splice(0, 1), 'file-missing-or-changed:fuze-rust-host.exe'],
  ]) {
    const e = evidence(); change(e)
    const report = assessReadiness(build, e.binaries, e.windows, e.policy)
    assert.equal(report.staticChecksPassed, false); assert.ok(report.blockers.includes(reason))
  }
})
test('simulated passing static checks never imply an installer, runtime load, or release is ready', () => {
  const e = evidence(), report = assessReadiness(build, e.binaries, e.windows, e.policy)
  assert.equal(report.staticChecksPassed, true)
  for (const name of ['releaseReady', 'installerVerified', 'runtimeLoadVerified', 'installRollbackVerified', 'appLaunched', 'systemChanged']) assert.equal(report[name], false)
})
test('invalid build identity and guessed policy values are rejected', () => {
  const e = evidence()
  assert.throws(() => assessReadiness({ ...build, revision: '../private' }, e.binaries, e.windows), /release-build-invalid/)
  assert.throws(() => assessReadiness(build, e.binaries, e.windows, { minimumVcRuntime: '1.26.0' }), /release-runtime-version-invalid/)
  assert.throws(() => assessReadiness(build, e.binaries, e.windows, { publisherSha256: 'automatic' }), /release-publisher-invalid/)
})
test('Windows command inspects non-executable fixtures, preserves inputs and refuses unsafe report writes', { skip: process.platform !== 'win32', timeout: 120000 }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'fuze-release-readiness-'))
  const bundle = path.join(root, 'payload with spaces')
  await mkdir(bundle)
  const bytes = peFixture(), buildText = JSON.stringify(build)
  for (const name of nativeFiles) await writeFile(path.join(bundle, name), bytes)
  await writeFile(path.join(bundle, 'build.json'), buildText)
  const command = fileURLToPath(new URL('../../scripts/check-rust-release-readiness.mjs', import.meta.url))
  const report = path.join(root, 'report.json')
  const run = destination => spawnSync(process.execPath, [command, '--bundle', bundle, '--report', destination], { encoding: 'utf8', windowsHide: true, timeout: 30000 })
  const first = run(report)
  assert.equal(first.status, 2, first.stderr)
  const result = JSON.parse(await readFile(report, 'utf8'))
  assert.equal(result.revision, build.revision); assert.equal(result.appLaunched, false)
  assert.equal(result.releaseReady, false); assert.equal(result.staticChecksPassed, false)
  const saved = await readFile(report)
  assert.equal(run(report).status, 1)
  assert.deepEqual(await readFile(report), saved)
  const inside = path.join(bundle, '..evidence')
  await mkdir(inside)
  assert.equal(run(path.join(inside, 'report.json')).status, 1)
  assert.deepEqual(await readdir(inside), [])
  for (const name of nativeFiles) assert.deepEqual(await readFile(path.join(bundle, name)), bytes)
  assert.equal(await readFile(path.join(bundle, 'build.json'), 'utf8'), buildText)
  // Keep this tiny, clearly named isolated fixture; no recursive cleanup of an input directory.
})
