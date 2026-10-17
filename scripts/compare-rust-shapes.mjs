// Offline serialized A/B: one Rust binary, model, adapter and fixture content.
import path from 'node:path'
import { writeFile } from 'node:fs/promises'
import { prepare, output, runOcr } from './rust-prototype-tools.mjs'
const args = await prepare()
const results = []
const previous = process.env.FUZE_RUST_DML_SHAPE
try {
  for (const mode of ['cpu', 'dynamic', 'padded512', 'fixed512']) {
    process.env.FUZE_RUST_DML_SHAPE = mode === 'cpu' ? 'fixed512' : mode
    const result = runOcr(args, mode === 'cpu' ? 'cpu' : 'dml', path.join(output, 'shape-tensors'))
    results.push({ mode, ...result })
    const times = (result.inference || []).flatMap(row => row.runs_ms).sort((a,b) => a-b)
    console.log(JSON.stringify({ mode, passed: result.passed, adapter: result.adapter, init_ms: result.init_ms,
      warm_ms: result.warm_ms, median_ms: times[Math.floor(times.length/2)],
      cpu_ms: (result.inference || []).flatMap(row => row.cpu_ms).reduce((a,b) => a+(b||0),0),
      frames_ms: result.samples?.map(s => s.runs_ms), error: result.error }))
  }
} finally {
  if (previous === undefined) delete process.env.FUZE_RUST_DML_SHAPE
  else process.env.FUZE_RUST_DML_SHAPE = previous
}
await writeFile(path.join(output, 'shape-comparison.json'), JSON.stringify(results, null, 2))
if (results.some(r => !r.exit_ok || !r.passed)) process.exitCode = 1
