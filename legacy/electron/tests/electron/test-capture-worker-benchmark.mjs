// Controlled pixels only. This microbenchmark does not measure game FPS or desktop IPC.
import fs from 'node:fs'
import vm from 'node:vm'
import { createHash } from 'node:crypto'
import assert from 'node:assert/strict'

const width = 1024, height = 576, count = 120
const source = fs.readFileSync(new URL('../../public/capture-worker.js', import.meta.url), 'utf8')
async function measure(every, optimized) {
  let pixels = new Uint8ClampedArray(width * height * 4).fill(128)
  let callback = () => {}, clock = 0
  const video = { readyState: 2, videoWidth: width, videoHeight: height,
    play: async () => {}, requestVideoFrameCallback: fn => { callback = fn } }
  const sandbox = { Uint32Array, performance: { now: () => clock },
    navigator: { mediaDevices: { getDisplayMedia: async () => ({ active: true }) } },
    document: { createElement: tag => tag === 'video' ? video : { width, height,
      getContext: () => ({ drawImage() {}, getImageData: () => ({ data: pixels.slice() }) }) } } }
  vm.runInNewContext(source, sandbox)
  await sandbox.fuzeCapture.start()
  let transferredBytes = 0, accepted = 0, hash = ''
  const start = performance.now()
  for (let i = 0; i < count; i++) {
    if (i % every === 0) pixels[100003] = (pixels[100003] + 1) % 256
    clock += 500; callback()
    if (optimized) {
      const result = sandbox.fuzeCapture.read()
      if (result.pixels) { transferredBytes += result.pixels.byteLength; structuredClone(result); accepted++ }
    } else {
      const result = structuredClone({ width, height, pixels: pixels.slice().buffer })
      transferredBytes += result.pixels.byteLength
      const next = createHash('sha1').update(Buffer.from(result.pixels)).digest('hex')
      if (next !== hash) { accepted++; hash = next }
    }
  }
  const elapsedMs = performance.now() - start
  pixels = null
  return { elapsedMs: Math.round(elapsedMs * 100) / 100, transferredBytes, accepted }
}
const results = []
for (const [scenario, every] of [['static', 120], ['ten-percent-changing', 10], ['all-changing', 1]]) {
  const before = await measure(every, false)
  const after = await measure(every, true)
  assert.equal(after.accepted, before.accepted, 'Deduplication must preserve each change')
  results.push({ scenario, frames: count, before, after,
    avoidedPayloadPercent: Math.round((1 - after.transferredBytes / before.transferredBytes) * 1000) / 10 })
}
console.log(JSON.stringify({ scope: 'Synthetic RGBA buffer comparison and structuredClone, not actual IPC, game FPS, or whole-app CPU', width, height, results }, null, 2))
