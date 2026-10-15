import { spawnSync } from 'node:child_process'
import { pathToFileURL } from 'node:url'
import path from 'node:path'
import { lstat, readFile, readdir } from 'node:fs/promises'

// Findings contain locations and rule names only, never matched credential material.
export function inspectUpload(name, buffer) {
  const file = name.replaceAll('\\', '/').replace(/^\//, '')
  const findings = []
  if (/(^|\/)(?:\.env(?:\..*)?|\.netrc|id_rsa|id_ed25519|personal-history-v1\.json|augment-journal-v1\.json|observations\.json|account-history\.json|window-(?:main|champions)\.json)$/.test(file)
    && !file.endsWith('.example')) findings.push({ file, line: 0, rule: 'private-file' })
  if (/(?:^|\/)(?:aramgg_client-data|FuzeRustPreview|user-profile|\.runtime|release-smoke)\//.test(file)
    || /\.(?:pfx|p12|key|keystore|log)$/i.test(file)) findings.push({ file, line: 0, rule: 'runtime-or-key-file' })
  if (buffer.includes(0)) return findings
  const source = buffer.toString('utf8')
  const rules = [
    ['private-key', /-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----/g],
    ['github-token', /\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{50,})\b/g],
    ['api-key', /\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{32,}\b/g],
    ['aws-key', /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/g],
    ['jwt', /\beyJ[A-Za-z0-9_-]{12,}\.[A-Za-z0-9_-]{12,}\.[A-Za-z0-9_-]{12,}\b/g],
    ['credential-url', /https?:\/\/[^\s/:]+:[A-Za-z0-9._~%+=-]{8,}@[^\s'"`)]+/g],
    ['literal-credential', /(?:api[_-]?key|access[_-]?token|refresh[_-]?token|remoting[_-]?auth[_-]?token|client[_-]?secret|password|_authToken)["']?\s*[:=]\s*["']([A-Za-z0-9_+/.=-]{20,})["']/gi],
  ]
  for (const [rule, pattern] of rules) for (const match of source.matchAll(pattern)) {
    // Reviewed toy strings used in existing LCU tests, never a blanket tests-directory exclusion.
    const toys = new Set(['manual-lock-token', 'manual-log-token', 'private-lcu-token', 'private-token',
      'private-entitlements-token', 'lockfile-token', 'log-token', 'url-token', 'stale-private-token',
      'first-token', 'second-token', 'status-token', 'test-token-123', 'PRIVATE_TOKEN_AND_IDENTITY_MUST_NOT_ESCAPE'])
    if (file.startsWith('tests/unit/')) {
      if (rule === 'literal-credential' && toys.has(match[1])) continue
      if (rule === 'credential-url') {
        try { const url = new URL(match[0]); if (['127.0.0.1', 'localhost'].includes(url.hostname) && toys.has(url.password)) continue } catch { /* Report unparseable credentials. */ }
      }
    }
    findings.push({ file, line: source.slice(0, match.index).split('\n').length, rule })
  }
  return findings
}

function git(args, input) {
  const result = spawnSync('git', args, { input, maxBuffer: 192 * 1024 * 1024, windowsHide: true })
  if (result.status !== 0) throw new Error('Unable to inspect Git content')
  return result.stdout
}
export async function checkUploads(args = []) {
  let findings = [], checked = 0
  if (args.includes('--packaged')) {
    const asar = await import('@electron/asar')
    const archive = path.resolve('build/win-unpacked/resources/app.asar')
    for (const file of asar.listPackage(archive)) {
      const name = file.replaceAll('\\', '/').replace(/^\//, '')
      const info = asar.statFile(archive, path.normalize(name), false)
      if ('files' in info || info.link) continue
      checked++; findings.push(...inspectUpload(name, asar.extractFile(archive, path.normalize(name), false)))
    }
    const resources = path.dirname(archive)
    const scanResource = async relative => {
      const target = path.join(resources, relative)
      const info = await lstat(target)
      if (info.isSymbolicLink()) throw new Error('Unexpected packaged resource link')
      if (info.isDirectory()) { for (const child of await readdir(target)) await scanResource(path.join(relative, child)); return }
      checked++; findings.push(...inspectUpload(relative, await readFile(target)))
    }
    for (const relative of ['client-data', 'recognition-catalog.json', 'app-update.yml']) {
      try { await scanResource(relative) } catch (error) { if (error.code !== 'ENOENT') throw error }
    }
  } else {
    const files = git(['ls-files', '--stage', '-z']).toString('utf8').split('\0').filter(Boolean).map(line => {
      const [, mode, id, name] = /^(\d+) ([a-f0-9]+) \d\t([\s\S]+)$/.exec(line) || []
      if (!id || mode !== '100644' && mode !== '100755') throw new Error('Unsupported Git entry; review links and submodules before upload')
      return { id, name }
    })
    const rangeIndex = args.indexOf('--range')
    if (rangeIndex >= 0) {
      const range = args[rangeIndex + 1]
      if (!range || range.startsWith('-')) throw new Error('A commit range is required')
      const history = git(['rev-list', '--objects', range, '--']).toString('utf8').split('\n')
      for (const row of history) {
        const separator = row.indexOf(' ')
        if (separator > 0) files.push({ id: row.slice(0, separator), name: row.slice(separator + 1) })
      }
    }
    const unique = [...new Map(files.map(file => [`${file.id}:${file.name}`, file])).values()]
    const data = git(['cat-file', '--batch'], unique.map(file => file.id).join('\n') + '\n')
    let cursor = 0
    for (const file of unique) {
      const end = data.indexOf(10, cursor)
      const [, type, length] = data.subarray(cursor, end).toString().split(' ')
      const bytes = Number(length)
      if (end < 0 || !Number.isSafeInteger(bytes)) throw new Error('Incomplete Git inspection')
      if (type === 'blob') { checked++; findings.push(...inspectUpload(file.name, data.subarray(end + 1, end + 1 + bytes))) }
      cursor = end + 1 + bytes + 1
    }
  }
  findings = [...new Map(findings.map(f => [`${f.file}:${f.line}:${f.rule}`, f])).values()]
  console.log(JSON.stringify({ checked, findings }, null, 2))
  if (findings.length) process.exitCode = 1
}
if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await checkUploads(process.argv.slice(2))
}
