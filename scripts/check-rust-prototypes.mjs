// Offline fixture validation. Never imports the application or calls game APIs.
import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const output = path.join(root, 'rust/artifacts')
await mkdir(output, { recursive: true })
const lines = (await readFile(path.join(root, 'resources/paddleocr/rec/inference.yml'), 'utf8')).split(/\r?\n/)
const start = lines.findIndex(line => /^\s*character_dict:\s*$/.test(line))
if (start < 0) throw new Error('Missing model dictionary')
const dictionary = ['']
for (const line of lines.slice(start + 1)) {
  const match = /^\s*- (.*)$/.exec(line)
  if (!match) { if (line.trim()) break; continue }
  let value = match[1]
  if (value.startsWith("'") && value.endsWith("'")) value = value.slice(1, -1).replaceAll("''", "'")
  else if (value.startsWith('"') && value.endsWith('"')) value = JSON.parse(value)
  dictionary.push(value)
}
// The bundled PP-OCRv5 model has 18,385 classes: blank + YAML characters + trailing space.
// Paddle's use_space_char appends this class after the dictionary, not before it.
dictionary.push(' ')
const fixtures = path.join(root, 'tests/fixtures/augment-ocr')
const manifest = JSON.parse(await readFile(path.join(fixtures, 'manifest.json'), 'utf8'))
const catalog = Object.fromEntries(manifest.flatMap(row => row.expectedNames.map((name, i) => [name, row.expectedIds[i]])))
const dictionaryPath = path.join(output, 'dictionary.json'), catalogPath = path.join(output, 'catalog.json')
await writeFile(dictionaryPath, JSON.stringify(dictionary))
await writeFile(catalogPath, JSON.stringify(catalog))
function run(name, args, runtimeDirectory) {
  // Only the child receives the trusted runtime dependency directory; no global PATH changes.
  const env = { ...process.env }
  if (runtimeDirectory) {
    const key = Object.keys(env).find(key => key.toLowerCase() === 'path') || 'Path'
    env[key] = runtimeDirectory + path.delimiter + (env[key] || '')
  }
  const child = spawnSync(path.join(root, 'rust/target/release', name + '.exe'), args, { cwd: root, env, windowsHide: true, timeout: 120000, encoding: 'utf8' })
  // Native-library diagnostics can contain absolute paths. Do not copy stderr to reports.
  const line = child.stdout?.trim().split(/\r?\n/).findLast(line => line.startsWith('{'))
  const phase = /fuze-ocr: phase=(\d+)/.exec(child.stderr || '')?.[1]
  const report = line ? JSON.parse(line) : { error: 'prototype-process-failed', phase: phase ? Number(phase) : null }
  return { ...report, exit_code: child.status, exit_ok: child.status === 0 }
}
const ort = process.env.FUZE_RUST_ORT_DLL || path.join(root, 'node_modules/onnxruntime-node/bin/napi-v6/win32/x64/onnxruntime.dll')
const ocr = run('fuze-ocr', [ort, path.join(root, 'resources/paddleocr/rec/inference.onnx'), dictionaryPath, catalogPath, fixtures], path.dirname(ort))
const snapshotPath = path.join(output, 'snapshot.json')
// Use the real partial-fixture OCR result, preserving the two missing positions.
const snapshot = ocr.samples?.[1]?.snapshot
let overlay = { exit_ok: false, error: 'ocr-snapshot-unavailable' }
if (snapshot) {
  await writeFile(snapshotPath, JSON.stringify(snapshot))
  overlay = run('fuze-native-overlay', ['--smoke', path.join(output, 'native-overlay.png'), snapshotPath])
}
await writeFile(path.join(output, 'validation.json'), JSON.stringify({ ocr, overlay, liveGameValidated: false, gpuValidated: false }, null, 2))
console.log(JSON.stringify({ ocr, overlay, liveGameValidated: false, gpuValidated: false }, null, 2))
if (!ocr.exit_ok || !ocr.passed || !overlay.exit_ok) process.exitCode = 1
