import { beforeEach, describe, expect, it, vi } from 'vitest'
const mocks = vi.hoisted(() => ({ process: vi.fn(), manual: vi.fn(), installed: vi.fn(), get: vi.fn() }))
vi.mock('../../src/main/services/lcu/installed-client-discovery.ts', () => ({ discoverLcuAuthFromInstalledDirectory: mocks.installed }))
vi.mock('../../src/main/services/lcu/process-auth-discovery.ts', () => ({ discoverLcuAuthFromProcess: mocks.process }))
vi.mock('../../src/main/services/lcu/manual-directory-auth.ts', () => ({ discoverLcuAuthFromManualDirectory: mocks.manual }))
vi.mock('../../src/main/modules/app-store.ts', () => ({ default: { get: mocks.get } }))
vi.mock('../../src/main/modules/logger.ts', () => ({ default: { info: vi.fn(), debug: vi.fn(), error: vi.fn() } }))
import { getLcuToken } from '../../src/main/services/lcu/token-loader.ts'
beforeEach(() => { vi.clearAllMocks(); mocks.installed.mockResolvedValue([null, null, null]); mocks.get.mockReturnValue('D:\\old-install'); mocks.process.mockResolvedValue([null, null, null]); mocks.manual.mockResolvedValue([null, null, null]) })
describe('automatic client discovery takes precedence', () => {
  it('uses verified installation metadata before requiring a manual path', async () => {
    mocks.installed.mockResolvedValue(['fixture', '50000', null])
    expect(await getLcuToken()).toEqual(['fixture', '50000', null])
    expect(mocks.manual).not.toHaveBeenCalled()
  })
  it('does not use a configured old directory when a running client provides auth', async () => {
    mocks.process.mockResolvedValue(['fixture', '50000', null])
    expect(await getLcuToken()).toEqual(['fixture', '50000', null])
    expect(mocks.manual).not.toHaveBeenCalled()
  })
  it('uses a manual directory only after automatic discovery fails', async () => {
    await getLcuToken()
    expect(mocks.manual).toHaveBeenCalledWith('D:\\old-install')
    expect(mocks.process.mock.invocationCallOrder[0]).toBeLessThan(mocks.manual.mock.invocationCallOrder[0])
  })
  it('does not require a manual directory for automatic discovery', async () => {
    mocks.get.mockReturnValue(undefined)
    expect(await getLcuToken()).toEqual([null, null, null])
    expect(mocks.process).toHaveBeenCalledOnce()
    expect(mocks.manual).not.toHaveBeenCalled()
  })
})
