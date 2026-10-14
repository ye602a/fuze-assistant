import { writeFile } from 'node:fs/promises'
import path from 'node:path'
import { output, prepare, run, runOcr } from './rust-prototype-tools.mjs'
const args = await prepare()
const ocr = runOcr(args)
const snapshotPath = path.join(output, 'snapshot.json')
const snapshot = ocr.samples?.[1]?.snapshot
let overlay = { exit_ok: false, error: 'ocr-snapshot-unavailable' }
if (snapshot) {
  await writeFile(snapshotPath, JSON.stringify(snapshot))
  overlay = run('fuze-native-overlay', ['--smoke', path.join(output, 'native-overlay.png'), snapshotPath])
}
const report = { ocr, overlay, liveGameValidated: false, gpuValidated: false }
await writeFile(path.join(output, 'validation.json'), JSON.stringify(report, null, 2))
console.log(JSON.stringify(report))
if (!ocr.exit_ok || !ocr.passed || !overlay.exit_ok) process.exitCode = 1
