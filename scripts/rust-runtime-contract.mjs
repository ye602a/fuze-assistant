import { createHash } from 'node:crypto'
import { atLeast, inspectPe } from './rust-release-readiness.mjs'
export const crtNames = Object.freeze(['vcruntime140.dll','vcruntime140_1.dll','msvcp140.dll','msvcp140_1.dll','msvcp140_2.dll','msvcp140_atomic_wait.dll','concrt140.dll'])
export const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

export function makeRuntimeContract({ policyBytes, evidence, revision, frontend, onnxVersion, binaries }) {
  const policy = JSON.parse(policyBytes)
  const vendorNames = ['DirectML.dll','dxcompiler.dll','dxil.dll','onnxruntime.dll']
  if (!['slint','win32'].includes(frontend) || !/^[a-f0-9]{12}$/.test(revision)
      || policy.schema !== 1 || policy.architecture !== 'x64' || policy.onnxRuntimeVersion !== onnxVersion
      || policy.basis !== 'supported-baseline-not-lowest-abi-version'
      || JSON.stringify(Object.keys(policy.thirdPartySha256 || {}).sort()) !== JSON.stringify(vendorNames)
      || !atLeast(policy.minimumVersion, policy.redistributable?.version)
      || policy.minimumVersion !== policy.redistributable.version
      || !/^[a-f0-9]{64}$/.test(policy.redistributable.sha256)
      || !/^https:\/\/download\.visualstudio\.microsoft\.com\/download\/pr\/[a-zA-Z0-9/-]+\/VC_redist\.x64\.exe$/.test(policy.redistributable.url)) throw new Error('runtime-policy-invalid')
  if (evidence?.schema !== 1 || evidence.sourceClean !== true || evidence.revision !== revision || evidence.frontend !== frontend
      || evidence.policySha256 !== sha256(policyBytes) || evidence.completed !== true
      || !atLeast(policy.minimumVersion, evidence.linkerVersion)
      || !/^19\.\d+\.\d+\.\d+$/.test(evidence.compilerVersion)
      || !atLeast(policy.minimumVersion, evidence.compilerVersion.replace(/^19\./, '14.'))) throw new Error('runtime-build-evidence-invalid')
  const own = ['fuze-rust-host.exe','fuze-ocr-worker.exe','fuze-update.exe', ...(frontend === 'slint' ? ['fuze-ui-lab.exe'] : [])]
  const required = new Set(), hashes = {}
  for (const name of [...own, ...Object.keys(policy.thirdPartySha256)]) {
    const bytes = binaries[name]
    if (!bytes || sha256(bytes) !== (own.includes(name) ? evidence.binaries?.[name] : policy.thirdPartySha256[name])) throw new Error('runtime-artifact-changed')
    const pe = inspectPe(bytes)
    if (pe.architecture !== 'x64') throw new Error('runtime-artifact-architecture')
    for (const dependency of [...pe.imports, ...pe.delayImports]) {
      if (/^(vcruntime|msvcp|concrt)\d/.test(dependency)) {
        if (!crtNames.includes(dependency)) throw new Error('runtime-new-dependency-needs-review')
        required.add(dependency)
      }
    }
    hashes[name] = sha256(bytes)
  }
  if (!required.size) throw new Error('runtime-dependencies-empty')
  return { schema: 1, architecture: 'x64', minimumVersion: policy.minimumVersion,
    basis: policy.basis, compilerVersion: evidence.compilerVersion, linkerVersion: evidence.linkerVersion,
    policySha256: evidence.policySha256, requiredDlls: [...required].sort(), binaries: hashes,
    redistributable: policy.redistributable }
}

export function installCommand() {
  // The helper stays ZIP-only: existing managed updaters reject new root files.
  return '@echo off\r\ncd /d "%~dp0"\r\nif not exist "%~dp0Install-local.ps1" (\r\n  echo Please use Install-local.cmd from the original preview ZIP.\r\n  pause\r\n  exit /b 2\r\n)\r\npowershell.exe -NoProfile -File "%~dp0Install-local.ps1"\r\nexit /b %errorlevel%\r\n'
}
