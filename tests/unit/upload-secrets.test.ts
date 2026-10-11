import { describe, expect, it } from 'vitest'
import { inspectUpload } from '../../scripts/check-secrets.mjs'
describe('upload credential guard', () => {
  it('blocks credential formats without returning matched material', () => {
    const secret = ['gh', 'p_', 'x'.repeat(36)].join('')
    const result = inspectUpload('src/example.ts', Buffer.from(`const value = '${secret}'`))
    expect(result).toEqual([{ file: 'src/example.ts', line: 1, rule: 'github-token' }])
    expect(JSON.stringify(result)).not.toContain(secret)
  })
  it('blocks runtime account files and private keys, including in tests', () => {
    expect(inspectUpload('match-history/personal-history-v1.json', Buffer.from('{}'))).toHaveLength(1)
    const header = ['-----BEGIN ', 'PRIVATE KEY-----'].join('')
    expect(inspectUpload('tests/unit/a.ts', Buffer.from(header))[0].rule).toBe('private-key')
  })
  it('allows env templates and only the reviewed loopback toy credentials in unit fixtures', () => {
    expect(inspectUpload('.env.local.example', Buffer.from('API_KEY=replace-me'))).toEqual([])
    const toy = ['https://riot:', 'private-token', '@127.0.0.1:1234'].join('')
    expect(inspectUpload('tests/unit/example.test.ts', Buffer.from(toy))).toEqual([])
    expect(inspectUpload('src/example.ts', Buffer.from(toy))[0].rule).toBe('credential-url')
  })
})
