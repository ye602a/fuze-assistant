import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { root, output, prepare, childReport, runOcr } from './rust-prototype-tools.mjs'
const args = await prepare()
const tensors = path.join(output, 'tensors')
const rustCpu = runOcr(args, 'cpu', tensors)
const nodeCpu = rustCpu.exit_ok ? childReport(process.execPath, [path.join(root, 'scripts/benchmark-rust-node.mjs'), ...args.slice(1,4), tensors, 'cpu']) : { exit_ok: false }
const automatic = runOcr(args, 'auto')
let rustDml = null, nodeDml = null
if (process.env.FUZE_RUST_GPU_TEST === '1' && rustCpu.exit_ok) {
  rustDml = runOcr(args, 'dml', path.join(output, 'tensors-dml'))
  nodeDml = childReport(process.execPath, [path.join(root, 'scripts/benchmark-rust-node.mjs'), ...args.slice(1,4), tensors, 'dml'])
}
const report = { rustCpu, nodeCpu, automatic, rustDml, nodeDml, scope: 'same-model-same-tensors-inference-only', liveGameValidated: false }
await writeFile(path.join(output, 'comparison.json'), JSON.stringify(report, null, 2))
console.log(JSON.stringify(report))
if (!rustCpu.exit_ok || !rustCpu.passed || !nodeCpu.exit_ok || !nodeCpu.passed || !automatic.exit_ok || !automatic.passed) process.exitCode = 1
