// Explicit developer command; never called by startup, install, update or packaging.
import { readFile, lstat, realpath, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { nativeFiles, inspectPe, assessReadiness } from './rust-release-readiness.mjs'

async function main() {
  if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('release-windows-x64-required')
  const options = {}, allowed = new Set(['--bundle', '--report', '--publisher-sha256', '--minimum-vc-runtime'])
  const args = process.argv.slice(2)
  for (let i = 0; i < args.length; i += 2) {
    if (!allowed.has(args[i]) || !args[i + 1] || options[args[i]]) throw new Error('release-arguments')
    options[args[i]] = args[i + 1]
  }
  if (!options['--bundle']) throw new Error('release-arguments')
  const bundle = await realpath(options['--bundle'])
  const bounded = async (name, limit) => {
    const file = path.join(bundle, name), info = await lstat(file)
    if (!info.isFile() || info.isSymbolicLink() || info.size > limit
        || (await realpath(file)).toLowerCase() !== file.toLowerCase()) throw new Error('release-file-boundary')
    return readFile(file)
  }
  const build = JSON.parse(await bounded('build.json', 16384))
  const binaries = []
  for (const name of nativeFiles) {
    const bytes = await bounded(name, 128 * 1024 * 1024)
    binaries.push({ name, sha256: createHash('sha256').update(bytes).digest('hex'), ...inspectPe(bytes) })
  }
  const script = fileURLToPath(new URL('./inspect-rust-release-windows.ps1', import.meta.url))
  const powershell = path.join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe')
  // A Node child of PowerShell 7 inherits its module path; Windows PowerShell needs its own.
  const env = { ...process.env }
  delete env.PSModulePath
  const child = spawnSync(powershell, ['-NoProfile', '-NonInteractive', '-File', script], {
    input: JSON.stringify({ bundle }), env, encoding: 'utf8', windowsHide: true, timeout: 120000, maxBuffer: 262144,
  })
  if (child.status !== 0) throw new Error('release-windows-inspection-failed')
  const report = assessReadiness(build, binaries, JSON.parse(child.stdout), {
    publisherSha256: options['--publisher-sha256'], minimumVcRuntime: options['--minimum-vc-runtime'],
  })
  const body = JSON.stringify(report, null, 2) + '\n'
  if (options['--report']) {
    const destination = path.resolve(options['--report']), parent = await realpath(path.dirname(destination))
    const relative = path.relative(bundle, parent)
    if (!(relative === '..' || relative.startsWith('..' + path.sep) || path.isAbsolute(relative))) throw new Error('release-report-inside-bundle')
    await writeFile(destination, body, { flag: 'wx' })
  }
  process.stdout.write(body)
  // 0 means the narrow static checks passed; never that the product is ready to release.
  process.exitCode = report.staticChecksPassed ? 0 : 2
}
main().catch(error => {
  console.error(/^release-[a-z0-9-]+$/.test(error?.message) ? error.message : 'release-readiness-inspection-failed')
  process.exitCode = 1
})
