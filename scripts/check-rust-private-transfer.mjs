// Explicit network acceptance only. No package creation, installation, game or UI access.
// Existing Git credentials stay in memory and enter the verifier through anonymous stdin.
import {spawn, spawnSync} from 'node:child_process'
import {access, lstat, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {fileURLToPath} from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const allowedErrors = new Set(['update-access-or-release', 'update-network', 'update-cancelled-or-timeout', 'update-cancelled',
  'update-range', 'update-http-status', 'update-redirect-policy', 'update-redirect-limit', 'update-map-hash',
  'update-final-hash', 'delta-final-hash', 'validation-real-range-required', 'validation-full-fallback',
  'validation-root-must-be-new', 'validation-baseline', 'credential-format', 'update-release-assets',
  'update-release-channel', 'update-manifest-policy', 'update-auth-policy', 'update-size', 'update-write', 'update-stage'])
async function main() {
  const args = process.argv.slice(2)
  if (args.length !== 3) throw new Error('arguments')
  const [verifier, baseline, output] = args.map(value => path.resolve(value))
  const artifacts = path.join(root, 'rust', 'artifacts') + path.sep
  if (path.basename(verifier).toLowerCase() !== 'fuze-update-verify.exe'
    || !output.startsWith(artifacts) || !path.basename(output).startsWith('private-transfer-')) throw new Error('paths')
  for (const file of [verifier, baseline, baseline + '.blockmap']) {
    const stat = await lstat(file)
    if (!stat.isFile() || stat.isSymbolicLink() || stat.size === 0) throw new Error('inputs')
  }
  try { await access(output); throw new Error('output-exists') } catch (error) { if (error.code !== 'ENOENT') throw error }
  const env = {...process.env, GIT_TERMINAL_PROMPT: '0', GCM_INTERACTIVE: 'Never', GIT_TRACE: '0', GIT_CURL_VERBOSE: '0'}
  for (const key of Object.keys(env)) if (/^GIT_TRACE/i.test(key)) delete env[key]
  env.GIT_TRACE = '0'
  delete env.FUZE_UPDATE_VERIFY_TRACE
  delete env.FUZE_UPDATE_TEST_ACCESS
  const auth = spawnSync('git', ['credential', 'fill'], {
    cwd: root, input: 'protocol=https\nhost=github.com\npath=ye602a/fuze-assistant.git\n\n',
    env, windowsHide: true, timeout: 15000, maxBuffer: 65536,
  })
  let secret
  try {
    if (auth.status !== 0 || !auth.stdout) throw new Error('credential-unavailable')
    const prefix = Buffer.from('password=')
    for (let start = 0; start < auth.stdout.length;) {
      let end = auth.stdout.indexOf(10, start)
      if (end < 0) end = auth.stdout.length
      let line = auth.stdout.subarray(start, end)
      if (line.at(-1) === 13) line = line.subarray(0, -1)
      if (line.subarray(0, prefix.length).equals(prefix)) {
        if (secret) throw new Error('credential-format')
        secret = Buffer.from(line.subarray(prefix.length))
      }
      start = end + 1
    }
    if (!secret || secret.length > 512 || !secret.length
      || !secret.every(b => b === 95 || b >= 48 && b <= 57 || b >= 65 && b <= 90 || b >= 97 && b <= 122)) throw new Error('credential-format')
  } catch (error) { secret?.fill(0); throw error }
  finally { auth.stdout?.fill(0); auth.stderr?.fill(0) }
  console.log('Starting isolated Rust transfer verification; no installation.')
  let timer
  try {
    const child = spawn(verifier, ['--transport-private', output, baseline, baseline + '.blockmap'], {
      cwd: root, env: {...env, FUZE_UPDATE_VERIFY_TRACE: '1'}, windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'],
    })
    const buffers = [], errors = []
    let bytes = 0, terminationReason
    for (const [stream, target] of [[child.stdout, buffers], [child.stderr, errors]]) {
      stream.on('data', chunk => {
        bytes += chunk.length
        if (bytes > 65536) { terminationReason = 'validation-output-limit'; child.kill() }
        else target.push(chunk)
      })
    }
    child.stdin.on('error', () => {})
    child.stdin.end(secret, () => secret.fill(0))
    timer = setTimeout(() => { terminationReason = 'validation-deadline'; child.kill() }, 620000)
    const code = await new Promise((resolve, reject) => { child.on('error', reject); child.on('close', resolve) })
    const lines = Buffer.concat(errors).toString('utf8').trim().split(/\r?\n/)
    const transport = {api_requests: 0, cdn_requests: 0, api_headers_ms: 0, cdn_headers_ms: 0, body_ms: 0, received_bytes: 0}
    for (const line of lines) {
      const match = /^update-transfer role=(api|cdn) ranged=(true|false) headers_ms=(\d{1,10}) body_ms=(\d{1,10}) bytes=(\d{1,10})$/.exec(line)
      if (!match) continue
      transport[`${match[1]}_requests`]++
      transport[`${match[1]}_headers_ms`] += Number(match[3])
      transport.body_ms += Number(match[4])
      transport.received_bytes += Number(match[5])
    }
    if (code !== 0) {
      const error = lines.at(-1)
      const network = lines.map(line => /^update-network stage=(open|send|receive|available|read) code=(\d{1,10})$/.exec(line))
        .filter(Boolean).map(match => ({stage: match[1], code: Number(match[2])})).slice(0, 8)
      const delta = lines.map(line => /^validation-delta bytes=(\d{1,15}) requests=(\d{1,10}) ms=(\d{1,10})$/.exec(line)).find(Boolean)
      const deltaVerified = delta ? {bytes: Number(delta[1]), requests: Number(delta[2]), ms: Number(delta[3])} : undefined
      const budget = lines.map(line => /^update-budget scope=(caller|request) ms=(\d{1,10})$/.exec(line))
        .filter(Boolean).map(match => ({scope: match[1], ms: Number(match[2])})).slice(0, 8)
      const report = {success: false, error: terminationReason ?? (allowedErrors.has(error) ? error : 'private-transfer-verification-failed'), network, budget, deltaVerified,
        transport, installed: false, game_access: false, credential_persisted: false}
      try { await writeFile(path.join(output, 'transfer-report.json'), JSON.stringify(report, null, 2), {flag: 'wx'}) }
      catch (writeError) { if (writeError.code !== 'ENOENT') throw writeError }
      console.log(JSON.stringify(report))
      process.exitCode = 1
      return
    }
    const raw = JSON.parse(Buffer.concat(buffers).toString('utf8'))
    const report = {transport}
    for (const key of ['package_bytes', 'real_range_bytes', 'full_bytes', 'delta_received_bytes', 'fallback_received_bytes', 'delta_range_requests', 'delta_ms', 'total_ms']) {
      if (!Number.isSafeInteger(raw[key]) || raw[key] < 0) throw new Error('report')
      report[key] = raw[key]
    }
    for (const key of ['private_download', 'final_hashes_verified', 'full_fallback']) {
      if (raw[key] !== true) throw new Error('report')
      report[key] = true
    }
    for (const key of ['installed', 'game_access', 'credential_persisted']) {
      if (raw[key] !== false) throw new Error('report')
      report[key] = false
    }
    if (!/^[0-9a-f]{12}$/.test(raw.revision)) throw new Error('report')
    report.revision = raw.revision
    await writeFile(path.join(output, 'transfer-report.json'), JSON.stringify(report, null, 2), {flag: 'wx'})
    console.log(JSON.stringify(report))
  } finally { clearTimeout(timer); secret?.fill(0) }
}
main().catch(() => { console.error('Private transport verification could not complete; no installation performed.'); process.exitCode = 1 })
