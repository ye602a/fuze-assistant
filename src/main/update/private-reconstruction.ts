import { createWriteStream } from 'node:fs'
import { open } from 'node:fs/promises'
import { Readable, Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import { CancellationError, DigestTransform, configureRequestUrl, type CancellationToken, type ProgressInfo } from 'builder-util-runtime'
import type { BlockMap } from 'builder-util-runtime/out/blockMapApi.js'
import { computeOperations, OperationKind } from 'electron-updater/out/differentialDownloader/downloadPlanBuilder.js'
import type { OutgoingHttpHeaders, RequestOptions } from 'node:http'
import type { PrivateUpdateExecutor } from './private-update-executor.ts'

export function validateBlockmap(value: BlockMap, size: number): BlockMap {
  const file = value?.files?.[0]
  if (value?.version !== '2' || value.files.length !== 1 || !file || file.offset !== 0 || file.name !== 'file' ||
    !Array.isArray(file.sizes) || !Array.isArray(file.checksums) || !file.sizes.length || file.sizes.length > 100000 ||
    file.sizes.length !== file.checksums.length || file.sizes.some(n => !Number.isSafeInteger(n) || n <= 0) ||
    file.checksums.some(s => typeof s !== 'string' || !s.length || s.length > 128) ||
    file.sizes.reduce((sum, n) => sum + n, 0) !== size) throw new Error('Invalid update blockmap')
  return value
}

// Keep the SDK's block plan and SHA-512 validation, but own stream lifetime so COPY is cancellable too.
export async function reconstructPrivateUpdate(options: {
  oldMap: BlockMap; newMap: BlockMap; oldFile: string; destination: string; url: URL; sha512: string; size: number
  executor: PrivateUpdateExecutor; headers: OutgoingHttpHeaders | null; token: CancellationToken
  onProgress: (progress: ProgressInfo) => void
}) {
  const { token, executor } = options
  const check = () => { if (token.cancelled) throw new CancellationError() }
  check()
  const tasks = computeOperations(options.oldMap, options.newMap, { info() {}, warn() {}, error() {} })
  const total = tasks.filter(t => t.kind === OperationKind.DOWNLOAD).reduce((sum, t) => sum + t.end - t.start, 0)
  let active: Readable | undefined, transferred = 0, lastTransferred = 0, lastReport = Date.now(), written = 0
  const start = lastReport
  const controller = new AbortController()
  const cancel = () => { active?.destroy(); controller.abort() }
  const baseline = await open(options.oldFile, 'r')
  token.once('cancel', cancel)
  if (token.cancelled) cancel()
  const chunks = async function* () {
    for (const task of tasks) {
      check()
      const length = task.end - task.start
      let received = 0
      if (task.kind === OperationKind.COPY) {
        while (received < length) {
          check()
          const buffer = Buffer.allocUnsafe(Math.min(64 * 1024, length - received))
          const { bytesRead } = await baseline.read(buffer, 0, buffer.length, task.start + received)
          check()
          if (!bytesRead) throw new Error('Update baseline truncated')
          received += bytesRead
          yield buffer.subarray(0, bytesRead)
        }
        continue
      } else {
        const requestOptions: RequestOptions = { headers: { ...options.headers, range: `bytes=${task.start}-${task.end - 1}` } }
        configureRequestUrl(options.url, requestOptions)
        active = await new Promise<Readable>((resolve, reject) => {
          let incoming: Readable | undefined
          const request = executor.createRequest(requestOptions, response => {
            // Electron's declaration omits its Node readable stream methods.
            const stream = new Readable().wrap(response as unknown as NodeJS.ReadableStream)
            incoming = stream
            response.once('aborted', () => stream.destroy(new Error('Update response interrupted')))
            stream.once('close', () => { if (!stream.readableEnded) request.abort() })
            resolve(stream)
          })
          request.on('error', error => { incoming?.destroy(error); reject(error) })
          request.end()
        })
      }
      try {
        check()
        for await (const data of active) {
          check()
          const chunk = data as Buffer
          received += chunk.length
          if (received > length) throw new Error('Update range size mismatch')
          if (task.kind === OperationKind.DOWNLOAD) {
            transferred += chunk.length
            const now = Date.now()
            if (now - lastReport >= 1000 || transferred === total) {
              options.onProgress({ total, transferred, delta: transferred - lastTransferred,
                percent: total ? transferred / total * 100 : 100, bytesPerSecond: transferred / Math.max(1, (now - start) / 1000) })
              lastReport = now; lastTransferred = transferred
            }
          }
          yield chunk
        }
        check()
        if (received !== length) throw new Error('Update range size mismatch')
      } finally { active.destroy(); active = undefined }
    }
  }
  const limit = new Transform({ transform(chunk: Buffer, _encoding, callback) {
    written += chunk.length
    callback(written > options.size ? new Error('Update size mismatch') : null, chunk)
  } })
  try {
    await pipeline(Readable.from(chunks()), limit, new DigestTransform(options.sha512),
      createWriteStream(options.destination), { signal: controller.signal })
    check()
    if (written !== options.size) throw new Error('Update size mismatch')
  } finally {
    token.removeListener('cancel', cancel)
    active?.destroy()
    await baseline.close()
  }
}
