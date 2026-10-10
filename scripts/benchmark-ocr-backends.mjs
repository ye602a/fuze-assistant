import fs from 'node:fs/promises'
import path from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const output = path.resolve(process.argv[2] || path.join(root, '../../work/ocr-benchmark-' + Date.now()))
const modes = (process.argv[3] || 'cpu,webgpu1,mixed1,cpu1').split(',')
const rounds = Math.max(2, Math.min(30, Number(process.argv[4]) || 9))
const allowed = ['cpu', 'cpu1', 'dml', 'mixed', 'mixed1', 'webgpu', 'webgpu1']
if (modes.some(mode => !allowed.includes(mode))) throw new Error('Unknown benchmark mode')
await fs.mkdir(output, { recursive: true })
const report = { createdAt: new Date().toISOString(), rounds, note: 'Offline fixtures; no screen capture, game, LCU, or FPS validation. First round excluded from warm metrics. CPU milliseconds are cumulative process CPU time, not system CPU percentage.', results: [] }
for (const [index, mode] of modes.entries()) {
  const directory = path.join(output, `${index + 1}-${mode}`)
  await fs.mkdir(directory, { recursive: false })
  const log = await fs.open(path.join(directory, 'run.log'), 'w')
  const started = Date.now()
  let timedOut = false
  const child = spawn(process.execPath, ['--require', path.join(root, 'scripts/ocr-benchmark-provider.cjs'), path.join(root, 'tests/electron/test-augment-ocr-fixtures.js')], {
    cwd: root, windowsHide: true, stdio: ['ignore', log.fd, log.fd],
    env: { ...process.env, FUZE_BENCHMARK_MODE: mode, FUZE_BENCHMARK_DIRECTORY: directory,
      ARAMGG_OCR_BENCHMARK_ROUNDS: String(rounds), ARAMGG_OCR_BENCHMARK_OUTPUT: path.join(directory, 'frames.json') },
  })
  const timer = setTimeout(() => { timedOut = true; child.kill() }, Math.max(90000, rounds * 12000))
  let code
  try {
    code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', resolve) })
  } finally { clearTimeout(timer); await log.close() }
  const result = { mode, code, timedOut, elapsedMs: Date.now() - started }
  if (code === 0) {
    const { records } = JSON.parse(await fs.readFile(path.join(directory, 'frames.json'), 'utf8'))
    const warm = records.slice(6), sorted = warm.map(value => value.wallMs).sort((a, b) => a - b)
    Object.assign(result, { validatedFrames: records.length, warmFrames: warm.length,
      meanWallMs: warm.reduce((sum, value) => sum + value.wallMs, 0) / warm.length,
      meanCpuMs: warm.reduce((sum, value) => sum + value.cpuMs, 0) / warm.length,
      p95WallMs: sorted[Math.ceil(sorted.length * .95) - 1] })
  }
  report.results.push(result)
  await fs.writeFile(path.join(output, 'summary.json'), JSON.stringify(report, null, 2))
  console.log(JSON.stringify(result))
}
if (report.results.some(result => result.code !== 0)) process.exitCode = 1
