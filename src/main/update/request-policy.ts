import { URL } from 'node:url'
import type { OutgoingHttpHeaders } from 'node:http'

export const RELEASE_API = 'https://api.github.com/repos/ye602a/fuze-assistant/releases'
const CDN_HOSTS = new Set(['release-assets.githubusercontent.com', 'objects.githubusercontent.com'])

export function updateUrl(value: string | URL): URL {
  const url = new URL(value)
  const api = url.origin === 'https://api.github.com' &&
    /^\/repos\/ye602a\/fuze-assistant\/releases\/(?:latest|tags\/v\d+\.\d+\.\d+|assets\/\d+)$/.test(url.pathname) && !url.search
  if (url.protocol !== 'https:' || url.username || url.password || url.port || url.hash || (!api && !CDN_HOSTS.has(url.hostname))) {
    throw new Error('Update URL rejected')
  }
  return url
}

// No cookies, staging identity, or authorization reaches an asset CDN, even on redirects.
export function updateHeaders(value: string | URL, headers: OutgoingHttpHeaders): Record<string, string> {
  const url = updateUrl(value)
  const result: Record<string, string> = {}
  for (const [key, val] of Object.entries(headers)) {
    const name = key.toLowerCase()
    if (['accept', 'range', 'user-agent', 'cache-control'].includes(name) ||
      (name === 'authorization' && url.hostname === 'api.github.com')) { if (typeof val === 'string') result[name] = val }
  }
  if (url.hostname === 'api.github.com' && /\/assets\/\d+$/.test(url.pathname)) result.accept = 'application/octet-stream'
  return result
}
