import { build } from 'esbuild'
import { createRequire } from 'node:module'
import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import path from 'node:path'
import { spawn } from 'node:child_process'
const require = createRequire(import.meta.url)
await mkdir('build/private-update-test', { recursive: true })
await build({ entryPoints: ['tests/electron/private-update-fixture.ts'], bundle: true, platform: 'node', format: 'cjs',
  outfile: 'build/private-update-test/fixture.cjs', packages: 'external' })
const root = await mkdtemp(path.join(tmpdir(), 'fuze-update-fixture-'))
const env = { ...process.env, FUZE_UPDATE_FIXTURE_ROOT: root }
delete env.ELECTRON_RUN_AS_NODE
try {
  const child = spawn(require('electron'), [path.resolve('build/private-update-test/fixture.cjs')], { env, windowsHide: true, stdio: 'inherit' })
  const timeout = setTimeout(() => child.kill(), 90000)
  const code = await new Promise((resolve, reject) => { child.on('error', reject); child.on('exit', resolve) })
  clearTimeout(timeout)
  if (code !== 0) process.exitCode = 1
} finally {
  if (path.dirname(root) !== path.resolve(tmpdir()) || !path.basename(root).startsWith('fuze-update-fixture-')) throw new Error('Unexpected fixture path')
  await rm(root, { recursive: true, force: true, maxRetries: 3 })
}
