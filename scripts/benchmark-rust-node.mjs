// Isolated child: same ORT DLL, model and exported little-endian tensors; no application imports.
import fs from 'node:fs'
import path from 'node:path'
import * as ort from 'onnxruntime-node'
async function main() {
  const [model, dictionaryFile, catalogFile, directory, provider, sizing = 'dynamic'] = process.argv.slice(2)
  if (!['cpu', 'dml'].includes(provider)) throw new Error('invalid-provider')
  if (!['dynamic', 'pad512'].includes(sizing)) throw new Error('invalid-sizing')
  const width = sizing === 'dynamic' ? null : Number(sizing.replace(/\D/g, ''))
  const dictionary = JSON.parse(fs.readFileSync(dictionaryFile, 'utf8'))
  const catalog = JSON.parse(fs.readFileSync(catalogFile, 'utf8'))
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, 'manifest.json'), 'utf8'))
  if (!manifest.length || manifest.length > 96) throw new Error('invalid-manifest')
  const start = performance.now()
  const session = await ort.InferenceSession.create(model, {
    executionProviders: [provider === 'dml' ? { name: 'dml', deviceId: 0 } : 'cpu'], intraOpNumThreads: 1, interOpNumThreads: 1, executionMode: 'sequential',
    enableMemPattern: provider === 'cpu',
    extra: { session: { intra_op: { allow_spinning: '0' }, inter_op: { allow_spinning: '0' } } },
  })
  const init_ms = performance.now() - start, samples = []
  let passed = true
  try {
    for (const [index, entry] of manifest.entries()) {
      if (!/^\d+-[012]\.f32le$/.test(entry.file) || !Number.isInteger(entry.width) || entry.width < 16 || entry.width > 1024) throw new Error('invalid-input')
      const file = path.join(directory, entry.file)
      if (fs.statSync(file).size !== 3 * 48 * entry.width * 4) throw new Error('invalid-tensor')
      const bytes = fs.readFileSync(file), data = new Float32Array(bytes.length / 4)
      for (let i = 0; i < data.length; i++) data[i] = bytes.readFloatLE(i * 4)
      if (width && width < entry.width) throw new Error('input-overflows-fixed-width')
      const padded = width ? new Float32Array(3 * 48 * width) : data
      if (width) for (let row = 0; row < 3 * 48; row++) padded.set(data.subarray(row * entry.width, (row + 1) * entry.width), row * width)
      const tensor = new ort.Tensor('float32', padded, [1, 3, 48, width || entry.width])
      const feeds = { [session.inputNames[0]]: tensor }
      const first = performance.now()
      await session.run(feeds)
      const first_ms = performance.now() - first
      const runs_ms = [], cpu_ms = []
      for (let run = 0; run < 3; run++) {
        const start = performance.now(), cpu = process.cpuUsage()
        const out = Object.values(await session.run(feeds))[0]
        runs_ms.push(performance.now() - start)
        const used = process.cpuUsage(cpu); cpu_ms.push((used.user + used.system) / 1000)
        if (out.dims.length !== 3 || out.dims[0] !== 1 || out.dims[2] !== dictionary.length) throw new Error('invalid-shape')
        let previous = 0, text = '', score = 0, count = 0
        for (let t = 0; t < out.dims[1]; t++) {
          const offset = t * dictionary.length; let best = 0
          for (let c = 1; c < dictionary.length; c++) if (out.data[offset + c] >= out.data[offset + best]) best = c
          if (best !== 0 && best !== previous) { text += dictionary[best]; score += out.data[offset + best]; count++ }
          previous = best
        }
        const id = count && score / count >= .75 ? catalog[text.replace(/\s/g, '')] ?? null : null
        passed &&= id === entry.expected_id
      }
      samples.push({ index, width: width || entry.width, content_width: entry.width, first_ms, runs_ms, cpu_ms })
    }
  } finally { await session.release() }
  console.log(JSON.stringify({ runtime: 'node', provider, sizing, init_ms, samples, passed }))
  if (!passed) process.exitCode = 1
}
main().catch(() => { console.log(JSON.stringify({ error: 'node-benchmark-failed' })); process.exitCode = 1 })
