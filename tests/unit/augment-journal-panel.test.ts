// @vitest-environment happy-dom
import { flushPromises, mount, type VueWrapper } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import { afterEach, describe, expect, it, vi } from 'vitest'
import AugmentJournalPanel from '../../src/renderer/components/AugmentJournalPanel.vue'
import { messages } from '../../src/renderer/i18n/messages.ts'
const mocks = vi.hoisted(() => ({ read: vi.fn(), on: vi.fn(), unsubscribe: vi.fn() }))
vi.mock('../../src/renderer/native/electron-api.ts', () => ({ hasElectronAPI: () => true,
  electronAPI: { matchHistory: { getSelectionJournal: mocks.read }, events: { on: mocks.on } } }))
const saved = { success: true, data: { playerName: 'Fixture account', platformId: 'HN1', offline: true, sessions: [{
  gameId: 123, startedAt: 1000, endedAt: 5000, interrupted: false, capped: false, finalAugmentIds: [1],
  events: [{ observedAt: 2000, level: 3, kind: 'first', partial: true, cards: [{ id: 1, name: 'Left choice' }, null, { id: 3, name: 'Right choice' }] }],
}] } }
const wrappers: VueWrapper[] = []
const start = () => {
  mocks.on.mockReturnValue(mocks.unsubscribe)
  const wrapper = mount(AugmentJournalPanel, { global: { plugins: [createI18n({ legacy: false, locale: 'zh-CN', messages })] } })
  wrappers.push(wrapper)
  return wrapper
}
async function toggle(wrapper: VueWrapper, open: boolean) {
  ;(wrapper.element as HTMLDetailsElement).open = open
  await wrapper.trigger('toggle')
  await flushPromises()
}
afterEach(() => { wrappers.splice(0).forEach(wrapper => wrapper.unmount()); vi.resetAllMocks() })
describe('choice journal interface', () => {
  it('does not query until expanded and preserves the unread middle slot', async () => {
    mocks.read.mockResolvedValue(saved)
    const wrapper = start()
    await flushPromises()
    expect(mocks.read).not.toHaveBeenCalled()
    await toggle(wrapper, true)
    expect(mocks.read).toHaveBeenCalledOnce()
    expect(wrapper.findAll('.journal-card')).toHaveLength(0)
    const session = wrapper.find('.journal-session')
    ;(session.element as HTMLDetailsElement).open = true
    await session.trigger('toggle')
    expect(wrapper.findAll('.journal-card').map(card => card.text())).toEqual(['◇Left choice赛后拥有', '—此位置未识别', '◇Right choice'])
    expect(wrapper.text()).toContain('不代表在这次画面中选择')
    expect(wrapper.text()).toContain('上次记录的账号')
  })
  it('does not let nested expansion repeat the local query', async () => {
    mocks.read.mockResolvedValue(saved)
    const wrapper = start()
    await toggle(wrapper, true)
    await wrapper.find('.journal-session').trigger('toggle')
    expect(mocks.read).toHaveBeenCalledOnce()
  })
  it('ignores a delayed result after collapse and removes the event listener on unmount', async () => {
    let finish!: (value: typeof saved) => void
    mocks.read.mockReturnValue(new Promise(resolve => { finish = resolve }))
    const wrapper = start()
    await toggle(wrapper, true)
    await toggle(wrapper, false)
    finish(saved)
    await flushPromises()
    expect(wrapper.find('.journal-account').exists()).toBe(false)
    wrapper.unmount()
    expect(mocks.unsubscribe).toHaveBeenCalledOnce()
  })
  it('clears a previous account when refreshing fails to verify login', async () => {
    mocks.read.mockResolvedValue(saved)
    const wrapper = start()
    await toggle(wrapper, true)
    mocks.read.mockResolvedValue({ success: false, issue: 'login-required' })
    await wrapper.find('button').trigger('click')
    await flushPromises()
    expect(wrapper.find('.journal-account').exists()).toBe(false)
    expect(wrapper.find('[role="alert"]').text()).toContain('请先登录客户端')
  })
  it('uses a local-record error for unreadable files instead of an online-service error', async () => {
    mocks.read.mockResolvedValue({ success: false, issue: 'cache-unavailable' })
    const wrapper = start()
    await toggle(wrapper, true)
    expect(wrapper.find('[role="alert"]').text()).toContain('已有文件已保留')
  })
})
