import { afterEach, describe, expect, it, vi } from 'vitest'
import { EventEmitter } from 'node:events'
import https from 'node:https'
import { verifyDirectoryAuth } from '../../src/main/services/lcu/installed-client-discovery.ts'

describe('installation candidate read-only authentication probe', () => {
  afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers() })
  function response(statusCode: number, body: string) {
    const request = Object.assign(new EventEmitter(), { destroy: vi.fn() })
    const reply = Object.assign(new EventEmitter(), { statusCode, destroy: vi.fn() })
    const get = vi.spyOn(https, 'get').mockImplementation(((options, callback) => {
      queueMicrotask(() => { callback(reply); reply.emit('data', Buffer.from(body)); reply.emit('end') })
      return request
    }) as any)
    return { request, get }
  }
  it('accepts only an explicit 200 gameflow response on fixed loopback', async () => {
    const { get } = response(200, '"Lobby"')
    expect(await verifyDirectoryAuth(['toy', '50000', null])).toBe(true)
    expect(get).toHaveBeenCalledWith(expect.objectContaining({ hostname: '127.0.0.1',
      path: '/lol-gameflow/v1/gameflow-phase', port: 50000, agent: false }), expect.any(Function))
  })
  it.each([401, 404, 302, 500])('rejects status %i, including redirects', async status => {
    response(status, '"Lobby"')
    expect(await verifyDirectoryAuth(['toy', '50000', null])).toBe(false)
  })
  it.each(['{}', '"unknown-phase"', 'x'.repeat(257)])('rejects invalid or oversized bodies', async body => {
    response(200, body)
    expect(await verifyDirectoryAuth(['toy', '50000', null])).toBe(false)
  })
  it('bounds a stalled request and rejects invalid ports without connecting', async () => {
    vi.useFakeTimers()
    const request = Object.assign(new EventEmitter(), { destroy: vi.fn() })
    const get = vi.spyOn(https, 'get').mockReturnValue(request as any)
    expect(await verifyDirectoryAuth(['toy', '65536', null])).toBe(false)
    expect(get).not.toHaveBeenCalled()
    const pending = verifyDirectoryAuth(['toy', '50000', null])
    await vi.advanceTimersByTimeAsync(800)
    expect(await pending).toBe(false)
    expect(request.destroy).toHaveBeenCalledOnce()
  })
})
