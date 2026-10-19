// Developer-only static inspection. Never maps/loads a PE or invokes a bundled executable.
export const nativeFiles = Object.freeze([
  'fuze-rust-host.exe', 'fuze-ui-lab.exe', 'fuze-ocr-worker.exe', 'fuze-update.exe',
  'onnxruntime.dll', 'DirectML.dll', 'dxcompiler.dll', 'dxil.dll',
])
// https://learn.microsoft.com/en-us/windows/win32/debug/pe-format
export function inspectPe(bytes) {
  const fail = () => { throw new Error('release-pe-invalid') }
  const bound = (offset, size) => {
    if (!Number.isSafeInteger(offset) || offset < 0 || size < 0 || offset + size > bytes.length) fail()
    return offset
  }
  const u16 = offset => bytes.readUInt16LE(bound(offset, 2))
  const u32 = offset => bytes.readUInt32LE(bound(offset, 4))
  if (!Buffer.isBuffer(bytes) || bytes.length > 128 * 1024 * 1024 || u16(0) !== 0x5a4d) fail()
  const pe = u32(0x3c)
  if (u32(pe) !== 0x4550) fail()
  const machine = u16(pe + 4), count = u16(pe + 6), optionalSize = u16(pe + 20)
  const optional = pe + 24, magic = u16(optional)
  if (![0x10b, 0x20b].includes(magic) || count < 1 || count > 96) fail()
  bound(optional, optionalSize)
  const directoryStart = magic === 0x20b ? 112 : 96
  if (optionalSize < directoryStart || u32(optional + directoryStart - 4) > 16) fail()
  const directoryCount = u32(optional + directoryStart - 4)
  if (directoryStart + directoryCount * 8 > optionalSize) fail()
  const sections = []
  for (let i = 0; i < count; i++) {
    const at = optional + optionalSize + i * 40
    bound(at, 40)
    const raw = u32(at + 20), size = u32(at + 16), rva = u32(at + 12)
    bound(raw, size)
    sections.push({ raw, size, rva })
  }
  const offset = (rva, size = 1) => {
    const matches = sections.filter(s => rva >= s.rva && rva + size <= s.rva + s.size)
    if (matches.length !== 1) fail()
    return bound(matches[0].raw + rva - matches[0].rva, size)
  }
  const dllName = rva => {
    let name = ''
    for (let i = 0; i < 160; i++) {
      const value = bytes[offset(rva + i)]
      if (value === 0) {
        if (!/^[a-zA-Z0-9_-][a-zA-Z0-9_.-]*\.dll$/i.test(name)) fail()
        return name.toLowerCase()
      }
      if (value < 32 || value > 126) fail()
      name += String.fromCharCode(value)
    }
    fail()
  }
  const imports = (index, stride, nameAt, delay = false) => {
    if (directoryCount <= index) return []
    const entry = optional + directoryStart + index * 8
    const rva = u32(entry), size = u32(entry + 4)
    if (rva === 0 && size === 0) return []
    if (!rva || size < stride || size > stride * 513) fail()
    const names = []
    for (let i = 0; (i + 1) * stride <= size; i++) {
      const at = offset(rva + i * stride, stride)
      if (bytes.subarray(at, at + stride).every(value => value === 0)) return [...new Set(names)].sort()
      // Modern PE delay descriptors use RVA fields. Reject unsupported VA descriptors.
      if (delay && u32(at) !== 1) fail()
      names.push(dllName(u32(at + nameAt)))
    }
    fail()
  }
  return {
    architecture: machine === 0x8664 && magic === 0x20b ? 'x64' : 'unsupported',
    imports: imports(1, 20, 12), delayImports: imports(13, 32, 4, true),
  }
}

function version(value) {
  if (typeof value !== 'string' || !/^14\.\d{1,5}\.\d{1,5}\.\d{1,5}$/.test(value)) return null
  return value.split('.').map(Number)
}
export function atLeast(actual, minimum) {
  const a = version(actual), b = version(minimum)
  if (!a || !b) return false
  for (let i = 0; i < 4; i++) if (a[i] !== b[i]) return a[i] > b[i]
  return true
}

export function assessReadiness(build, binaries, windows, policy = {}) {
  if (!/^[a-f0-9]{12,40}$/.test(build?.revision) || !Number.isSafeInteger(build.sequence)
      || build.sequence <= 0 || build.uiFrontend !== 'slint') throw new Error('release-build-invalid')
  const publisher = policy.publisherSha256?.toLowerCase() || null
  const minimum = policy.minimumVcRuntime || null
  if (publisher && !/^[a-f0-9]{64}$/.test(publisher)) throw new Error('release-publisher-invalid')
  if (minimum && !version(minimum)) throw new Error('release-runtime-version-invalid')
  const blockers = []
  if (!publisher) blockers.push('publisher-identity-unconfigured')
  if (!minimum) blockers.push('vc-runtime-minimum-unrecorded')
  if (!windows?.x64Process) blockers.push('x64-inspector-required')
  const dependencies = new Set()
  const files = nativeFiles.map(name => {
    const binary = binaries.find(item => item.name === name)
    const observed = windows?.binaries?.find(item => item.name === name)
    const own = name.endsWith('.exe')
    if (!binary || !observed || binary.sha256 !== observed.sha256) {
      blockers.push(`file-missing-or-changed:${name}`)
      return { name, inspected: false }
    }
    if (binary.architecture !== 'x64') blockers.push(`architecture:${name}`)
    if (observed.signatureStatus !== 'Valid') blockers.push(`signature:${name}`)
    if (own && publisher && observed.signerSha256 !== publisher) blockers.push(`publisher:${name}`)
    if (own && !observed.timestampPresent) blockers.push(`timestamp:${name}`)
    for (const imported of [...binary.imports, ...binary.delayImports]) {
      if (/^(vcruntime|msvcp|concrt)\d.*\.dll$/.test(imported)) dependencies.add(imported)
    }
    return { ...binary, signatureStatus: observed.signatureStatus,
      signerSha256: observed.signerSha256, timestampPresent: observed.timestampPresent }
  })
  const runtime = [...dependencies].sort().map(name => {
    const installed = windows?.runtime?.find(item => item.name === name)
    const present = installed?.present === true
    if (!present) blockers.push(`vc-runtime-missing:${name}`)
    else {
      if (installed.architecture !== 'x64') blockers.push(`vc-runtime-architecture:${name}`)
      if (minimum && !atLeast(installed.version, minimum)) blockers.push(`vc-runtime-version:${name}`)
    }
    return { name, present, version: installed?.version || null, architecture: installed?.architecture || null,
      meetsDeclaredMinimum: minimum && present ? atLeast(installed.version, minimum) : null }
  })
  return {
    schema: 1, revision: build.revision, sequence: build.sequence,
    onnxRuntimeVersion: typeof build.runtime === 'string' && /^\d+\.\d+\.\d+$/.test(build.runtime) ? build.runtime : null,
    approvedPublisherSha256: publisher, minimumVcRuntime: minimum,
    files, runtime, staticChecksPassed: blockers.length === 0, blockers,
    // Static trust/dependency evidence is never a production release authorization.
    releaseReady: false, installerVerified: false, runtimeLoadVerified: false,
    installRollbackVerified: false, appLaunched: false, systemChanged: false,
    remaining: ['signed-installer', 'vendor-trust-policy', 'dependency-bootstrap',
      'clean-machine-install', 'runtime-load', 'upgrade-rollback-restart'],
  }
}
