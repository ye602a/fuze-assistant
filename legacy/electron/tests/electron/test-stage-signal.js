// Isolated transport fixture; never starts Electron or connects to a game.
import assert from 'node:assert/strict'
import https from 'node:https'
import { EventEmitter } from 'node:events'
import { PassThrough } from 'node:stream'
import { StageSignalProbe, readStageProbe, PROBE_PATHS } from '../../src/main/services/stage-signal.ts'

const original = https.get
const calls = []
let scenario = 'normal'
https.get = (options, callback) => {
  calls.push(options)
  assert.equal(options.hostname, '127.0.0.1')
  assert.equal(options.port, 2999)
  assert.equal(options.method, 'GET')
  assert.equal(options.agent, false)
  assert.equal(options.auth, undefined)
  assert.equal(options.headers, undefined)
  const request = new EventEmitter()
  request.destroy = () => {}
  setImmediate(() => {
    const response = new PassThrough()
    response.statusCode = scenario === 'redirect' ? 302 : 200
    response.headers = { location: 'https://not-requested.invalid/' }
    callback(response)
    if (scenario === 'redirect') return
    if (scenario === 'oversize') { response.end('x'.repeat(256 * 1024 + 1)); return }
    if (scenario === 'invalid') { response.end('not-json'); return }
    response.end(JSON.stringify(options.path === PROBE_PATHS[1]
      ? { level: 11, summonerName: 'PRIVATE_FIXTURE', augmentStage: 2 }
      : options.path === PROBE_PATHS[2] ? { Events: [] } : { paths: {} }))
  })
  return request
}
try {
  const probe = new StageSignalProbe()
  const result = await probe.inspect()
  assert.equal(calls.length, 3)
  assert.equal(result.report.actualStage, null)
  assert.equal(result.report.level, 11)
  assert.equal(JSON.stringify(result).includes('PRIVATE_FIXTURE'), false)
  for (const [mode, status] of [['redirect', 'http-error'], ['oversize', 'too-large'], ['invalid', 'invalid-json']]) {
    scenario = mode
    const before = calls.length
    const response = await readStageProbe(PROBE_PATHS[1])
    assert.equal(response.status, status)
    assert.equal(response.data, undefined)
    assert.equal(calls.length, before + 1)
  }
  console.log('Stage signal transport fixtures passed: loopback GET, no redirect, bounded responses, redaction, stage unknown')
} finally { https.get = original }
