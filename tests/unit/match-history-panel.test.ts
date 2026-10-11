// @vitest-environment happy-dom
import { flushPromises, mount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import MatchHistoryPanel from '../../src/renderer/components/MatchHistoryPanel.vue'
import { messages } from '../../src/renderer/i18n/messages.ts'
import type { HextechAramMatchHistoryPage, HextechAramMatchHistoryQueryResult } from '../../src/shared/ipc-contract.ts'

const mocks = vi.hoisted(() => ({ cached: vi.fn(), query: vi.fn(), on: vi.fn() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({
  hasElectronAPI: () => true,
  electronAPI: { matchHistory: { getCachedCurrent: mocks.cached, queryCurrent: mocks.query }, events: { on: mocks.on } },
}))
const page: HextechAramMatchHistoryPage = {
  playerName: 'Test saved account', platformId: 'HN1', source: 'cache', queriedAt: 1000,
  startIndex: 0, count: 10, returnedCount: 1, hasPrevious: false, hasMore: false,
  matches: [{ gameId: 1, gameCreation: 1000, gameDuration: 600, gameVersion: '16.20', championId: 103,
    kills: 1, deaths: 2, assists: 3, result: 'win', augments: [], items: [] }],
}
afterEach(() => vi.resetAllMocks())
const start = () => mount(MatchHistoryPanel, { global: { plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages })] } })

describe('history cache-first interface', () => {
  it('refreshes saved records after a post-game event without another online query', async () => {
    mocks.cached.mockResolvedValue({ success: true, data: { ...page, offline: true } })
    const unsubscribe = vi.fn()
    mocks.on.mockReturnValue(unsubscribe)
    const wrapper = start()
    await flushPromises()
    mocks.cached.mockResolvedValue({ success: true, data: { ...page, playerName: 'Updated saved account' } })
    mocks.on.mock.calls[0][1]()
    await flushPromises()
    expect(wrapper.text()).toContain('Updated saved account')
    expect(mocks.query).not.toHaveBeenCalled()
    wrapper.unmount()
    expect(unsubscribe).toHaveBeenCalledOnce()
  })
  it('renders cached records while refresh is pending and keeps them on provider failure', async () => {
    mocks.cached.mockResolvedValue({ success: true, data: page })
    let finish!: (value: HextechAramMatchHistoryQueryResult) => void
    mocks.query.mockReturnValue(new Promise((resolve) => { finish = resolve }))
    const wrapper = start()
    await flushPromises()
    expect(wrapper.text()).toContain('Test saved account')
    expect(wrapper.text()).toContain('正在补充最新记录')
    finish({ success: false, issue: 'service-unavailable' })
    await flushPromises()
    expect(wrapper.findAll('.recent-match')).toHaveLength(1)
    expect(wrapper.text()).toContain('在线战绩服务暂不可用')
    wrapper.unmount()
  })
  it('labels disconnected history explicitly and does not refresh it', async () => {
    mocks.cached.mockResolvedValue({ success: true, data: { ...page, offline: true, limited: true } })
    const wrapper = start()
    await flushPromises()
    expect(wrapper.text()).toContain('上次保存的账号记录')
    expect(wrapper.text()).toContain('不代表全部历史')
    expect(mocks.query).not.toHaveBeenCalled()
    wrapper.unmount()
  })
  it('removes old account results when an account switch invalidates the refresh', async () => {
    mocks.cached.mockResolvedValue({ success: true, data: page })
    mocks.query.mockResolvedValue({ success: false, issue: 'account-changed' })
    const wrapper = start()
    await flushPromises()
    expect(wrapper.findAll('.recent-match')).toHaveLength(0)
    expect(wrapper.text()).toContain('账号或区服已变化')
    wrapper.unmount()
  })
})
