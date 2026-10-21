// Test-process instrumentation only; never imported by the installed application.
const fs = require('node:fs')
const path = require('node:path')
const ort = require('onnxruntime-node')
const mode = process.env.FUZE_BENCHMARK_MODE
const output = process.env.FUZE_BENCHMARK_DIRECTORY
const allowed = ['cpu', 'cpu1', 'dml', 'mixed', 'mixed1', 'webgpu', 'webgpu1']
if (!allowed.includes(mode) || !output) throw new Error('Use scripts/benchmark-ocr-backends.mjs')
const create = ort.InferenceSession.create.bind(ort.InferenceSession)
const sessions = []
ort.InferenceSession.create = async (buffer, options) => {
  const model = sessions.length === 0 ? 'det' : 'rec'
  const backend = mode.startsWith('mixed') ? (model === 'det' ? 'dml' : 'cpu')
    : mode.replace(/1$/, '')
  const startedAt = performance.now()
  const session = await create(buffer, {
    ...options,
    executionProviders: [backend],
    intraOpNumThreads: mode.endsWith('1') ? 1 : 2,
    interOpNumThreads: 1,
    executionMode: 'sequential',
    enableMemPattern: backend === 'cpu',
    enableProfiling: process.env.FUZE_BENCHMARK_PROFILE === '1',
    profileFilePrefix: path.join(output, model),
  })
  const metrics = { model, backend, initMs: performance.now() - startedAt, runs: [] }
  sessions.push(metrics)
  const run = session.run.bind(session)
  session.run = async (feeds, ...args) => {
    const wall = performance.now(), cpu = process.cpuUsage()
    const result = await run(feeds, ...args)
    const usage = process.cpuUsage(cpu)
    metrics.runs.push({ wallMs: performance.now() - wall, cpuMs: (usage.user + usage.system) / 1000,
      shapes: Object.fromEntries(Object.entries(feeds).map(([key, tensor]) => [key, tensor.dims])) })
    return result
  }
  const release = session.release.bind(session)
  session.release = async () => {
    if (process.env.FUZE_BENCHMARK_PROFILE === '1') session.endProfiling()
    return release()
  }
  return session
}
process.on('exit', () => fs.writeFileSync(path.join(output, 'inference.json'), JSON.stringify({ mode, sessions }, null, 2)))
