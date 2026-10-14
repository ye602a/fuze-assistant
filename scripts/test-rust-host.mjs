import test from 'node:test'
import assert from 'node:assert/strict'
import { childReport, shouldHostFallback } from './rust-prototype-tools.mjs'
test('a hung owned test worker is killed and its output is reduced to a fixed error', () => {
  const result = childReport(process.execPath, ['-e', 'setInterval(()=>{},1000)'], 200)
  assert.equal(result.error, 'worker_timeout')
  assert.equal(result.exit_ok, false)
})
test('host does not repeat an internal fallback or conceal strict-mode/recognition failures', () => {
  assert.equal(shouldHostFallback('auto', { exit_ok:false }), true)
  assert.equal(shouldHostFallback('auto', { exit_ok:false, fallback_started:true }), false)
  assert.equal(shouldHostFallback('dml', { exit_ok:false }), false)
  assert.equal(shouldHostFallback('auto', { exit_ok:false, passed:false }), false)
  const result = childReport(process.execPath, ['-e', 'console.error("private-diagnostic-placeholder");process.exit(1)'])
  assert.equal(result.error, 'worker_failed')
  assert.ok(!JSON.stringify(result).includes('private-diagnostic-placeholder'))
})
