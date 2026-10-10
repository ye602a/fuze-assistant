import { describe, expect, it } from 'vitest'
import { connectionIsVerified, runtimeState, type RuntimeStatus } from '../../src/shared/runtime-status.ts'
const connection = { discovered: true, verifiedAt: 10000, phase: 'None', error: null }
const status = (changes: Partial<RuntimeStatus> = {}): RuntimeStatus => ({
  connection, automatic: true, ocr: 'ready', capture: { running: false }, ...changes,
} as RuntimeStatus)
describe('honest runtime status', () => {
  it('never treats discovered credentials or stale phase as a successful connection', () => {
    expect(connectionIsVerified({ ...connection, verifiedAt: null }, 10000)).toBe(false)
    expect(connectionIsVerified(connection, 31000)).toBe(false)
    for (const error of ['HTTP 401', 'HTTP 404', 'request-failed']) {
      expect(connectionIsVerified({ ...connection, error }, 11000)).toBe(false)
    }
  })
  it('distinguishes lobby, champion selection, loading and active recognition', () => {
    expect(runtimeState(status(), 11000)).toBe('lobby')
    expect(runtimeState(status({ connection: { ...connection, phase: 'ChampSelect' } }), 11000)).toBe('champSelect')
    expect(runtimeState(status({ connection: { ...connection, phase: 'GameStart' } }), 11000)).toBe('loadingGame')
    const inGame = status({ connection: { ...connection, phase: 'InProgress' } })
    expect(runtimeState(inGame, 11000)).toBe('starting')
    expect(runtimeState({ ...inGame, capture: { ...inGame.capture, running: true } }, 11000)).toBe('inGame')
    expect(runtimeState({ ...inGame, ocr: 'error' }, 11000)).toBe('ocrError')
    expect(runtimeState({ ...inGame, capture: { ...inGame.capture, error: 'capture-failed' } }, 11000)).toBe('captureError')
    expect(runtimeState({ ...inGame, automatic: false }, 11000)).toBe('paused')
  })
  it('does not call an uninitialized OCR engine a failure outside a match', () => {
    expect(runtimeState(status({ ocr: 'idle' }), 11000)).toBe('lobby')
  })
})
