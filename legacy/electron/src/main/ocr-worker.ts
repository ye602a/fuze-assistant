import { readFileSync } from 'node:fs'
import { ensureOnnxruntimeNativeDllPath } from './modules/onnxruntime-native-path.ts'

const port = process.parentPort
if (!port) throw new Error('OCR worker requires utilityProcess')
let service: { recognize(input: any): Promise<any> } | null = null
let busy = false
port.on('message', async ({ data: message }) => {
  if (busy) { port.postMessage({ id: message.id, error: 'OCR worker busy' }); return }
  busy = true
  try {
    if (message.action === 'init') {
      ensureOnnxruntimeNativeDllPath()
      const [{ PaddleOcrService }, ort] = await Promise.all([import('paddleocr'), import('onnxruntime-node')])
      const sessions: import('onnxruntime-node').InferenceSession[] = []
      const options = message.options
      service = await PaddleOcrService.createInstance({
        ort: { ...ort, InferenceSession: { create: async (buffer: ArrayBuffer) => {
          const session = await ort.InferenceSession.create(buffer, {
            executionProviders: [message.backend], intraOpNumThreads: 1, interOpNumThreads: 1,
            executionMode: 'sequential', enableMemPattern: message.backend === 'cpu',
            extra: { session: { intra_op: { allow_spinning: '0' }, inter_op: { allow_spinning: '0' } } },
          })
          sessions.push(session)
          return session
        } } },
        detection: { ...options.detection, modelBuffer: new Uint8Array(readFileSync(options.detModelPath)).buffer },
        recognition: { ...options.recognition, modelBuffer: new Uint8Array(readFileSync(options.recModelPath)).buffer },
      })
      // Execute representative shapes so shader compilation precedes the first card frame.
      for (const [index, session] of sessions.entries()) {
        for (const shape of index === 0 ? [[1, 3, 512, 448]] : [[1, 3, 48, 128], [1, 3, 48, 256]]) {
          await session.run({ [session.inputNames[0]!]: new ort.Tensor('float32', new Float32Array(shape.reduce((a, b) => a * b, 1)), shape) })
        }
      }
      port.postMessage({ id: message.id, result: { backend: message.backend, warmed: true } })
    } else if (message.action === 'recognize') {
      if (!service) throw new Error('OCR worker is not initialized')
      port.postMessage({ id: message.id, result: await service.recognize(message.input) })
    } else throw new Error('Unsupported OCR worker action')
  } catch (error) {
    port.postMessage({ id: message.id, error: error instanceof Error ? error.message : 'OCR worker failed' })
  } finally { busy = false }
})
