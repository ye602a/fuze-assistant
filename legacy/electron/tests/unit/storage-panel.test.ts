// @vitest-environment happy-dom
import { mount, flushPromises, type VueWrapper } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import StoragePanel from '../../src/renderer/components/StoragePanel.vue'
import { messages } from '../../src/renderer/i18n/messages.ts'
const mocks = vi.hoisted(() => ({ preview: vi.fn(), clean: vi.fn() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({ electronAPI: { storage: mocks } }))
const plan = { success: true, preview: { id: 'fixture-preview', checkedAt: 1, expiresAt: 10, skipped: 0, protectionUnknown: false,
  categories: [{ kind: 'recommendations', bytes: 1024, files: 8 }], reclaimableBytes: 512,
  candidates: [{ label: 'zh-CN · 1.0', kind: 'version', bytes: 512, files: 3 }] } }
let wrapper: VueWrapper
function start() { wrapper = mount(StoragePanel, { global: { plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages })] } }); return wrapper }
async function toggle(open: boolean) { (wrapper.element as HTMLDetailsElement).open = open; await wrapper.trigger('toggle'); await flushPromises() }
afterEach(() => { wrapper?.unmount(); vi.resetAllMocks() })
describe('storage management UI', () => {
  it('does no startup scan, shows a preview on expansion and only sends its ID on explicit cleanup', async () => {
    mocks.preview.mockResolvedValue(plan)
    mocks.clean.mockResolvedValue({ success: true, removedBytes: 512, removedFiles: 3 })
    start(); expect(mocks.preview).not.toHaveBeenCalled()
    await toggle(true)
    expect(wrapper.text()).toContain('zh-CN · 1.0')
    expect(mocks.clean).not.toHaveBeenCalled()
    await wrapper.find('.clean').trigger('click'); await flushPromises()
    expect(mocks.clean).toHaveBeenCalledExactlyOnceWith('fixture-preview')
    expect(wrapper.find('[role="status"]').text()).toContain('已释放')
    expect(wrapper.find('.clean').exists()).toBe(false)
  })
  it('ignores delayed scan responses after collapse', async () => {
    let finish!: (value: unknown) => void
    mocks.preview.mockReturnValue(new Promise(resolve => { finish = resolve }))
    start(); await toggle(true); await toggle(false)
    finish(plan); await flushPromises()
    expect(wrapper.find('.storage-body').exists()).toBe(false)
    expect(mocks.clean).not.toHaveBeenCalled()
  })
  it('invalidates the cleanup button after a changed or expired preview', async () => {
    mocks.preview.mockResolvedValue(plan); mocks.clean.mockResolvedValue({ success: false, issue: 'changed', removedFiles: 0 })
    start(); await toggle(true); await wrapper.find('.clean').trigger('click'); await flushPromises()
    expect(wrapper.find('[role="alert"]').text()).toContain('重新检查')
    expect(wrapper.find('.clean').exists()).toBe(false)
  })
  it('reports partial cleanup rather than claiming all files were removed', async () => {
    mocks.preview.mockResolvedValue(plan); mocks.clean.mockResolvedValue({ success: false, issue: 'cleanup-failed', removedFiles: 1, removedBytes: 20 })
    start(); await toggle(true); await wrapper.find('.clean').trigger('click'); await flushPromises()
    expect(wrapper.find('[role="status"]').text()).toContain('剩余项目未继续清理')
  })
})
