import { net, session, type ClientRequest, type IncomingMessage } from 'electron'
import { EventEmitter } from 'node:events'
import { CancellationError, type CancellationToken } from 'builder-util-runtime'
import { ElectronHttpExecutor } from 'electron-updater/out/electronHttpExecutor.js'
import type { OutgoingHttpHeaders, RequestOptions } from 'node:http'
import { updateHeaders, updateUrl } from './request-policy.ts'

// A fresh request at every redirect prevents native followRedirect from replaying credentials.
class UpdateRequest extends EventEmitter {
  private current: ClientRequest | null = null
  private finished = false
  private timer?: ReturnType<typeof setTimeout>
  constructor(private readonly initial: URL, private readonly headers: OutgoingHttpHeaders,
    private readonly response: (value: IncomingMessage) => void,
    private readonly remembered: Map<string, URL>) { super() }
  private touch() {
    if (this.finished) return
    if (this.timer) { this.timer.refresh(); return }
    this.timer = setTimeout(() => this.fail(new Error('Update request timeout')), 120000)
    this.timer.unref()
  }
  private close() {
    if (this.finished) return
    this.finished = true
    clearTimeout(this.timer)
    this.emit('close')
  }
  fail(error: Error) {
    if (this.finished) return
    this.close()
    this.current?.abort()
    this.emit('error', error)
  }
  abort() { this.fail(new CancellationError()) }
  end() {
    if (!this.finished) this.start(this.remembered.get(this.initial.href) || this.initial, 0)
  }
  private start(target: URL, redirects: number) {
    if (this.finished) return
    let url: URL
    try { url = updateUrl(target) } catch { this.fail(new Error('Update redirect rejected')); return }
    if (redirects > 5) { this.fail(new Error('Too many update redirects')); return }
    this.touch()
    const headers = updateHeaders(url, this.headers)
    const request = net.request({ url: url.href, method: 'GET', headers, redirect: 'manual',
      session: session.fromPartition('fuze-private-updater', { cache: false }), useSessionCookies: false })
    this.current = request
    request.on('error', () => { if (request === this.current) this.fail(new Error('Update request failed')) })
    request.on('redirect', (_status, _method, location) => {
      if (request !== this.current || this.finished) return
      this.current = null
      request.abort()
      try {
        const next = updateUrl(location)
        // A CDN cannot redirect back to an authenticated API request.
        if (url.hostname !== 'api.github.com' && next.hostname === 'api.github.com') throw new Error()
        if (this.initial.pathname.includes('/assets/') && next.hostname !== 'api.github.com') this.remembered.set(this.initial.href, next)
        this.start(next, redirects + 1)
      } catch { this.fail(new Error('Update redirect rejected')) }
    })
    request.on('response', response => {
      if (this.finished) { request.abort(); return }
      const range = typeof headers.range === 'string' ? /^bytes=(\d+)-(\d+)$/.exec(headers.range) : null
      const received = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(String(response.headers['content-range']))
      if (range && (response.statusCode !== 206 || !received || received[1] !== range[1] || received[2] !== range[2] || !Number.isSafeInteger(Number(received[3])) || Number(received[3]) <= Number(range[2]))) {
        this.fail(new Error('Update range response rejected')); return
      }
      response.on('data', () => this.touch())
      response.once('end', () => this.close())
      response.once('aborted', () => this.fail(new Error('Update response interrupted')))
      response.once('error', () => this.fail(new Error('Update response failed')))
      this.response(response)
    })
    request.end()
  }
}

export class PrivateUpdateExecutor extends ElectronHttpExecutor {
  private activeToken: CancellationToken | null = null
  private requests = new Set<UpdateRequest>()
  private redirects = new Map<string, URL>()
  private cancelRequests = () => { for (const request of this.requests) request.abort() }
  beginDownload(token: CancellationToken) {
    if (token.cancelled) throw new CancellationError()
    this.activeToken = token
    this.redirects.clear()
    token.once('cancel', this.cancelRequests)
  }
  resetRedirects() { this.redirects.clear() }
  endDownload() {
    this.activeToken?.removeListener('cancel', this.cancelRequests)
    this.activeToken = null
    this.redirects.clear()
  }
  override createRequest(options: RequestOptions, callback: (response: IncomingMessage) => void): ClientRequest {
    if (this.activeToken?.cancelled) throw new CancellationError()
    const url = updateUrl(`${options.protocol || 'https:'}//${options.hostname || options.host}${options.port ? ':' + options.port : ''}${options.path || '/'}`)
    if (options.method && options.method !== 'GET') throw new Error('Update method rejected')
    const request = new UpdateRequest(url, options.headers as OutgoingHttpHeaders || {}, callback,
      this.activeToken ? this.redirects : new Map())
    this.requests.add(request)
    request.once('close', () => this.requests.delete(request))
    // HttpExecutor uses only EventEmitter, end and abort on this request facade.
    return request as unknown as ClientRequest
  }
}
