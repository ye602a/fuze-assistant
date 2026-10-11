// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from 'vitest'
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import StageStatisticsPanel from '../../src/renderer/components/StageStatisticsPanel.vue'
import { stageMessages } from '../../src/renderer/i18n/stage-messages.ts'
const mocks = vi.hoisted(() => ({ get: vi.fn() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({ electronAPI: { stageStatistics: mocks } }))
const snapshot = { schema: 1, championId: 1, source: 'ARAMKit', region: 'unknown', patch: '16.20', dataDate: '2026-10-11', dataPath: 'data/16.20-20261011-abcdef123456', fetchedAt: 1000,
  rows: { all: [{ id: 1068, winRate: .52, pickRate: .1, samples: 400 }], 1: [{ id: 1068, winRate: .61, pickRate: .04, samples: 220 }], 2: [], 3: [], 4: [] } }
let wrapper: VueWrapper
function start() { wrapper = mount(StageStatisticsPanel, { props: { championId: 1, augments: [{ id: 1068, name: '示例海克斯' }] }, global: { plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages: { 'zh-CN': { stageStats: stageMessages['zh-CN'] } } })] } }); return wrapper }
async function toggle(open: boolean) { (wrapper.element as HTMLDetailsElement).open = open; await wrapper.trigger('toggle'); await flushPromises() }
afterEach(() => { wrapper?.unmount(); vi.resetAllMocks() })
describe('stage reference UI', () => {
  it('loads only on expansion, switches all four stages locally and does not fill missing stages', async () => {
    mocks.get.mockResolvedValue({ snapshot, cached: true }); start()
    expect(mocks.get).not.toHaveBeenCalled(); await toggle(true)
    expect(wrapper.text()).toContain('地区未标明'); expect(wrapper.text()).toContain('52.0%')
    await wrapper.findAll('.stages button')[1].trigger('click'); expect(wrapper.text()).toContain('61.0%')
    await wrapper.findAll('.stages button')[2].trigger('click'); expect(wrapper.text()).toContain('未用整体胜率填充')
    expect(wrapper.find('table').exists()).toBe(false); expect(mocks.get).toHaveBeenCalledExactlyOnceWith(1, false)
  })
  it('ignores late replies after collapse or champion change', async () => {
    let finish!: (value: unknown) => void
    mocks.get.mockReturnValue(new Promise(resolve => { finish = resolve }))
    start(); await toggle(true); await toggle(false); finish({ snapshot }); await flushPromises()
    expect(wrapper.text()).not.toContain('52.0%')
    mocks.get.mockReturnValue(new Promise(resolve => { finish = resolve }))
    await toggle(true); await wrapper.setProps({ championId: 2 }); finish({ snapshot }); await flushPromises()
    expect(wrapper.text()).not.toContain('52.0%'); expect(mocks.get).toHaveBeenCalledTimes(2)
  })
  it('keeps valid cached data when a manual refresh fails and warns about patch mismatch', async () => {
    mocks.get.mockResolvedValue({ snapshot, cached: true, stale: true, referencePatch: '16.19' }); start(); await toggle(true)
    expect(wrapper.text()).toContain('版本不同')
    mocks.get.mockResolvedValue({ snapshot, cached: true, stale: true, issue: 'unavailable' })
    await wrapper.find('.retry').trigger('click'); await flushPromises()
    expect(mocks.get).toHaveBeenLastCalledWith(1, true)
    expect(wrapper.text()).toContain('52.0%'); expect(wrapper.find('[role="alert"]').text()).toContain('暂不可用')
  })
})
