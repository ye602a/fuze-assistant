import { readFile, writeFile, mkdir } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
export const output = path.join(root, 'rust/artifacts')
export const ort = process.env.FUZE_RUST_ORT_DLL || path.join(root, 'node_modules/onnxruntime-node/bin/napi-v6/win32/x64/onnxruntime.dll')
export function runtimeEnv() {
  const env = { ...process.env }
  const key = Object.keys(env).find(key => key.toLowerCase() === 'path') || 'Path'
  env[key] = path.dirname(ort) + path.delimiter + (env[key] || '')
  return env
}
export async function prepare() {
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
  dictionary.push(' ') // Bundled PP-OCRv5: blank + YAML dictionary + trailing space.
  const fixtures = path.join(root, 'tests/fixtures/augment-ocr')
  const manifest = JSON.parse(await readFile(path.join(fixtures, 'manifest.json'), 'utf8'))
  const catalog = Object.fromEntries(manifest.flatMap(row => row.expectedNames.map((name, i) => [name, row.expectedIds[i]])))
  const dictionaryPath = path.join(output, 'dictionary.json'), catalogPath = path.join(output, 'catalog.json')
  await writeFile(dictionaryPath, JSON.stringify(dictionary))
  await writeFile(catalogPath, JSON.stringify(catalog))
  return [ort, path.join(root, 'resources/paddleocr/rec/inference.onnx'), dictionaryPath, catalogPath, fixtures]
}
export function childReport(command, args, timeout = 120000) {
  const child = spawnSync(command, args, { cwd: root, env: runtimeEnv(), windowsHide: true, timeout, encoding: 'utf8', maxBuffer: 2 * 1024 * 1024 })
  const line = child.stdout?.trim().split(/\r?\n/).findLast(line => line.startsWith('{'))
  const phase = /fuze-ocr: phase=(\d+)/.exec(child.stderr || '')?.[1]
  let report = { error: child.error?.code === 'ETIMEDOUT' ? 'worker_timeout' : 'worker_failed', phase: phase ? Number(phase) : null }
  try { if (line) report = JSON.parse(line) } catch { /* fixed error only */ }
  return { ...report, fallback_started: /fuze-ocr: fallback=1/.test(child.stderr || ''), exit_code: child.status, exit_ok: child.status === 0 && !child.error }
}
export function run(name, args) {
  if (!['fuze-ocr', 'fuze-native-overlay', 'fuze-capture'].includes(name)) throw new Error('Invalid prototype')
  return childReport(path.join(process.env.FUZE_RUST_BIN_DIR || path.join(root, 'rust/target/release'), name + '.exe'), args)
}
// Only auto may restart a crashed/timed-out native child once on CPU.
export function runOcr(args, mode = 'cpu', tensorDirectory) {
  if (!['cpu', 'dml', 'dml-strict', 'auto'].includes(mode)) throw new Error('Invalid backend')
  const launch = backend => run('fuze-ocr', [...args, backend, ...(tensorDirectory ? [tensorDirectory] : [])])
  const first = launch(mode)
  if (!shouldHostFallback(mode, first)) return first
  const cpu = launch('cpu')
  return { ...cpu, host_fallback: { count: 1, reason: first.error || 'worker_failed' } }
}
export function shouldHostFallback(mode, first) {
  return mode === 'auto' && !first.exit_ok && first.passed !== false && !first.fallback_started
}
