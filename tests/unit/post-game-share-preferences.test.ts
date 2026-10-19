import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { describe, expect, it } from 'vitest'

describe('post-game share production window policy', () => {
  it('does not restore or show the main window when post-game data is ready', async () => {
    const source = await readFile(
      path.join(process.cwd(), 'src/main/modules/app-config.ts'),
      'utf8',
    )
    const notificationBlock = source.slice(
      source.indexOf('async function prepareAndNotifyPostGameShare'),
      source.indexOf('async function resolveInProgressChampion'),
    )

    expect(notificationBlock).toContain('post-game-share-ready')
    expect(notificationBlock).not.toContain('.restore()')
    expect(notificationBlock).not.toContain('.show()')
    expect(notificationBlock).not.toContain('.showInactive()')
  })
})
