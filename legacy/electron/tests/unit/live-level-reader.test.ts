import { describe, expect, it, vi } from 'vitest'
import { LiveLevelReader, parseActivePlayerLevel } from '../../src/main/live-level-reader.ts'

describe('bounded live level telemetry', () => {
  it('accepts a numeric level only, without retaining identity', () => {
    expect(parseActivePlayerLevel({ level: 7, summonerName: 'private' })).toBe(7)
    for (const level of [null, '7', 0, 31, 7.1, NaN]) expect(parseActivePlayerLevel({ level })).toBeNull()
  })
  it('polls no more than once per three seconds and clears failed telemetry', async () => {
    const read = vi.fn().mockResolvedValueOnce(7).mockResolvedValueOnce(null)
    const reader = new LiveLevelReader(read)
    expect(await reader.poll(10000)).toEqual({ level: 7, checkedAt: 10000 })
    await reader.poll(11000); expect(read).toHaveBeenCalledTimes(1)
    expect(await reader.poll(13000)).toBeNull()
  })
  it('does not revive telemetry after stop while a request was in flight', async () => {
    let resolve!: (value: number) => void
    const reader = new LiveLevelReader(() => new Promise(r => { resolve = r }))
    const pending = reader.poll(10000)
    reader.reset(); resolve(12); await pending
    expect(reader.value).toBeNull()
  })
})
