// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import Panel from '../../src/renderer/components/AramBenchRecommendation.vue'
import { messages } from '../../src/renderer/i18n/messages.ts'

const api = vi.hoisted(() => ({ get: vi.fn(), events: new Map<string, (data: unknown) => void>() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({
  hasElectronAPI: () => true,
  electronAPI: { lcu: { getAramBenchRecommendation: api.get }, events: {
    on: (name: string, callback: (data: unknown) => void) => { api.events.set(name, callback); return () => api.events.delete(name) },
  } },
}))
const recommendation = (name: string, refreshable = true) => ({ refreshable, gameflowPhase: 'ChampSelect', status: 'ready',
  candidates: [{ championId: 1, name, source: 'bench', winRate: .5, pickRate: .1 }] })
const result = (name: string) => ({ success: true, recommendation: recommendation(name) })
let wrapper: VueWrapper
function start(preview = recommendation('当前快照')) {
  wrapper = mount(Panel, { props: { previewRecommendation: preview }, global: {
    plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages })],
  } })
}
beforeEach(() => { vi.useFakeTimers(); api.get.mockReset(); api.events.clear() })
afterEach(() => { wrapper?.unmount(); vi.clearAllTimers(); vi.useRealTimers() })

describe('bench refresh ownership', () => {
  it('does not let an older request overwrite a newer selection snapshot and coalesces refreshes', async () => {
    let finish!: (value: unknown) => void
    api.get.mockImplementationOnce(() => new Promise(resolve => { finish = resolve })).mockResolvedValue(result('最新候选'))
    start(); await wrapper.setProps({ previewRecommendation: recommendation('切换后的快照') })
    await wrapper.setProps({ previewRecommendation: recommendation('再次切换') })
    expect(api.get).toHaveBeenCalledTimes(1)
    finish(result('旧候选')); await flushPromises()
    expect(api.get).toHaveBeenCalledTimes(2)
    expect(wrapper.text()).toContain('最新候选'); expect(wrapper.text()).not.toContain('旧候选')
  })
  it('does not display an obsolete request error after receiving a newer preview', async () => {
    let fail!: (error: Error) => void
    api.get.mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject }))
    start(); await wrapper.setProps({ previewRecommendation: recommendation('离线预览', false) })
    fail(new Error('旧错误')); await flushPromises()
    expect(wrapper.text()).toContain('离线预览'); expect(wrapper.text()).not.toContain('旧错误')
    expect(api.get).toHaveBeenCalledTimes(1)
  })
  it('leaves diagnostic preview on a real phase change and resumes live polling', async () => {
    api.get.mockResolvedValue(result('真实候选'))
    start(recommendation('预览候选', false)); expect(api.get).not.toHaveBeenCalled()
    api.events.get('game-phase-changed')?.({ phase: 'ChampSelect' }); await flushPromises()
    expect(wrapper.text()).toContain('真实候选'); expect(wrapper.text()).not.toContain('预览候选')
    await vi.advanceTimersByTimeAsync(3000); expect(api.get).toHaveBeenCalledTimes(2)
  })
  it('refreshes bench changes without changing the current champion or adding concurrent requests', async () => {
    api.get.mockResolvedValueOnce(result('候选甲')).mockResolvedValueOnce(result('候选乙'))
    start(); await flushPromises(); await vi.advanceTimersByTimeAsync(3000)
    expect(wrapper.text()).toContain('候选乙'); expect(wrapper.text()).not.toContain('候选甲')
    expect(api.get).toHaveBeenCalledTimes(2)
  })
})
