// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import Panel from '../../src/renderer/components/StageSignalDiagnostic.vue'
import { stageSignalMessages } from '../../src/renderer/i18n/stage-signal-messages.ts'
const api = vi.hoisted(() => ({ inspectStageSignal: vi.fn() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({ electronAPI: { diagnostics: api } }))
const report = { checkedAt: 1000, actualStage: null, level: 11, endpoints: [{ endpoint: 'player', status: 'ok', httpStatus: 200 }], schemaFields: [], playerFields: [], augmentEventCount: 0, truncated: false }
let wrapper: VueWrapper
const start = () => { wrapper = mount(Panel, { global: { plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages: { 'zh-CN': { stageSignal: stageSignalMessages['zh-CN'] } } })] } }) }
afterEach(() => { wrapper?.unmount(); vi.resetAllMocks() })
describe('manual stage inspection UI', () => {
  it('compares observations without promoting a stage and clears on scenario changes', async () => {
    const stable = { ...report, checkedAt: Date.now(), context: { revision: 1, status: 'stable' }, endpoints: ['schema', 'player', 'events'].map(endpoint => ({ endpoint, status: 'ok' })) }
    api.inspectStageSignal.mockResolvedValueOnce({ report: stable }).mockResolvedValue({ report: { ...stable, checkedAt: Date.now(), augmentEventCount: 1 } }); start()
    await wrapper.find('button').trigger('click'); await flushPromises()
    await wrapper.find('.compare-actions button').trigger('click')
    await wrapper.find('button').trigger('click'); await flushPromises()
    expect(wrapper.find('.comparison').exists()).toBe(true)
    expect(wrapper.text()).toContain('尚未确认')
    await wrapper.find('select').setValue('anvil')
    expect(wrapper.find('.comparison').exists()).toBe(false)
    expect(wrapper.find('.probe-result').exists()).toBe(false)
  })
  it('does not restore a pending observation after clearing it', async () => {
    let finish!: (value: unknown) => void
    api.inspectStageSignal.mockResolvedValueOnce({ report }).mockImplementationOnce(() => new Promise(resolve => { finish = resolve })); start()
    await wrapper.find('button').trigger('click'); await flushPromises()
    await wrapper.find('button').trigger('click')
    await wrapper.findAll('.compare-actions button')[1].trigger('click')
    finish({ report }); await flushPromises()
    expect(wrapper.find('.probe-result').exists()).toBe(false)
    expect(wrapper.find('button').attributes('disabled')).toBeUndefined()
  })
  it('does not read on mount and keeps unknown even after HTTP success', async () => {
    api.inspectStageSignal.mockResolvedValue({ report, retryAfterMs: 29000 }); start()
    expect(api.inspectStageSignal).not.toHaveBeenCalled()
    await wrapper.find('button').trigger('click'); await flushPromises()
    expect(wrapper.text()).toContain('实际领取轮次：尚未确认')
    expect(wrapper.text()).toContain('不用于推断领取轮次')
    expect(api.inspectStageSignal).toHaveBeenCalledTimes(1)
  })
  it('retains the explicitly timestamped report on cooldown without automatic retry', async () => {
    api.inspectStageSignal.mockResolvedValueOnce({ report, retryAfterMs: 29000 }).mockResolvedValue({ report: null, retryAfterMs: 22000 }); start()
    await wrapper.find('button').trigger('click'); await flushPromises()
    await wrapper.find('button').trigger('click'); await flushPromises()
    expect(wrapper.text()).toContain('22 秒'); expect(wrapper.text()).toContain('不会自动刷新')
    expect(api.inspectStageSignal).toHaveBeenCalledTimes(2)
  })
  it('disables duplicate clicks and ignores a response after unmount', async () => {
    let finish!: (value: unknown) => void
    api.inspectStageSignal.mockReturnValue(new Promise(resolve => { finish = resolve })); start()
    await wrapper.find('button').trigger('click'); expect(wrapper.find('button').attributes('disabled')).toBeDefined()
    wrapper.unmount(); finish({ report }); await flushPromises()
    expect(api.inspectStageSignal).toHaveBeenCalledTimes(1)
  })
})
