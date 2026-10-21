import { app, BrowserWindow, ipcMain, session } from 'electron'
import fs from 'node:fs/promises'
import path from 'node:path'
import http from 'node:http'
import https from 'node:https'
import { syncBuiltinESMExports } from 'node:module'

const ROUTES = ['/display', '/augment-overlay', '/floating-overlay', '/augment-side-panel']
const OFFLINE_ERROR = 'Release smoke: external network disabled'

/** Observe the real packaged startup; never synthesize renderer-ready or replace IPC. */
export function createReleaseSmoke() {
    const outputDir = process.env.ARAMGG_RELEASE_SMOKE_OUTPUT
    const dataDir = process.env.ARAMGG_RELEASE_SMOKE_DATA_DIR
    if (!outputDir || !path.isAbsolute(outputDir)) throw new Error('Smoke output must be an absolute path')
    if (!dataDir || !path.isAbsolute(dataDir)) throw new Error('Smoke data must be an absolute path')
    if (!app.isPackaged) throw new Error('Release smoke requires a packaged application')
    app.setPath('userData', dataDir)
    process.env.ARAMGG_ANALYTICS_ENABLED = 'false'

    // No remote data dependency or accidental production traffic in acceptance tests.
    const denyRequest = () => { throw new Error(OFFLINE_ERROR) }
    http.request = denyRequest
    http.get = denyRequest
    https.request = denyRequest
    https.get = denyRequest
    globalThis.fetch = async () => { throw new Error(OFFLINE_ERROR) }
    syncBuiltinESMExports()

    const startedAt = Date.now()
    const ready = new Set<number>()
    const failures: string[] = []
    const windows: BrowserWindow[] = []
    const fail = (message: string) => { failures.push(message) }
    ipcMain.on('renderer-ready', (event) => {
        if (event.senderFrame === event.sender.mainFrame) ready.add(event.sender.id)
    })
    app.on('browser-window-created', (_event, window) => {
        windows.push(window)
        if (process.env.ARAMGG_RELEASE_SMOKE_HIDDEN === '1') {
            window.show = () => {}
            window.showInactive = () => {}
            window.focus = () => {}
        }
        const contents = window.webContents
        contents.setBackgroundThrottling(false) // Hidden acceptance windows still need animation frames.
        contents.on('preload-error', (_event, preload, error) => fail(`Preload ${preload}: ${error.message}`))
        contents.on('render-process-gone', (_event, details) => fail(`Renderer crashed: ${details.reason}`))
        contents.on('did-fail-load', (_event, code, description, url, isMainFrame) => {
            if (isMainFrame) fail(`Page load ${url}: ${code} ${description}`)
        })
        contents.on('console-message', (details) => {
            if (details.level === 'error' && !details.message.includes(OFFLINE_ERROR) &&
                !details.message.includes('net::ERR_BLOCKED_BY_CLIENT')) {
                fail(`Renderer ${contents.getURL()}: ${details.message}`)
            }
        })
    })
    process.on('uncaughtException', (error) => fail(`Uncaught exception: ${error.stack || error.message}`))
    process.on('unhandledRejection', (error) => fail(`Unhandled rejection: ${String(error)}`))

    return {
        async run(initialize: () => Promise<unknown>) {
            const snapshots: unknown[] = []
            let timeout: ReturnType<typeof setTimeout> | undefined
            await fs.mkdir(outputDir, { recursive: true })
            session.defaultSession.webRequest.onBeforeRequest((details, callback) => {
                const url = new URL(details.url)
                callback({ cancel: url.protocol.startsWith('http') && url.hostname !== '127.0.0.1' })
            })
            try {
                await Promise.race([
                    (async () => {
                        const { default: store } = await import('../modules/app-store.ts')
                        store.set('itemSets.autoApplyAram', false)
                        await initialize()
                        const { getRecognitionDataStatus } = await import('../services/recognition-data-service.ts')
                        const recognitionData = getRecognitionDataStatus()
                        if (!recognitionData.count) throw new Error('Bundled independent recognition catalog is unavailable offline')
                        await fs.writeFile(path.join(outputDir, 'recognition-data.json'), JSON.stringify(recognitionData, null, 2))
                        while (ready.size < ROUTES.length) {
                            if (failures.length) throw new Error(failures.join('\n'))
                            await new Promise((resolve) => setTimeout(resolve, 50))
                        }
                        const main = windows.find((window) => !window.isDestroyed() &&
                            new URL(window.webContents.getURL()).hash === '#/display')
                        if (!main) throw new Error('Main window missing')
                        const preview = {
                            championId: 1, championName: 'Smoke champion', dataSource: 'test',
                            augments: [1, 2, 3].map((id) => ({
                                id: id === 1 ? 'mayhem:smoke-new' : id, augmentId: id === 1 ? 'mayhem:smoke-new' : id,
                                name: `Smoke augment ${id}`, detectedSlot: id - 1,
                                winRate: id === 1 ? null : 0.55, pickRate: id === 1 ? null : 0.2,
                                recommendScore: id === 1 ? null : 0.6, rarity: 'silver',
                                statistics: { status: id === 1 ? 'missing' : 'historical', patch: id === 1 ? null : '16.19' },
                                recognitionSource: 'arammayhem.com',
                            })),
                        }
                        const shown = await main.webContents.executeJavaScript(`(async () => {
                            const payload = ${JSON.stringify(preview)};
                            window.electronAPI.windows.showPopup(payload);
                            return window.electronAPI.diagnostics.testShowFloating(payload);
                        })()`)
                        if (!shown?.success || shown.skipped) throw new Error('Overlay diagnostic invocation failed')
                        // Auxiliary windows intentionally have no text until given an event payload.
                        await new Promise((resolve) => setTimeout(resolve, 500))
                        for (const route of ROUTES) {
                            const window = windows.find((candidate) => !candidate.isDestroyed() &&
                                new URL(candidate.webContents.getURL()).hash === `#${route}`)
                            if (!window || !ready.has(window.webContents.id)) throw new Error(`Window not ready: ${route}`)
                            // A real invoke must cross the packaged preload and registered main handler.
                            const snapshot = await window.webContents.executeJavaScript(`(async () => ({
                                route: location.hash,
                                mounted: !!document.querySelector('#app[data-v-app]'),
                                elements: document.querySelector('#app')?.childElementCount || 0,
                                text: (document.querySelector('#app')?.textContent || '').trim().slice(0, 160),
                                monitor: await window.electronAPI.lcu.getChampionMonitorState(),
                                locale: await window.electronAPI.locale.get()
                            }))()`)
                            if (!snapshot.mounted || !snapshot.elements || snapshot.text.length < 3 ||
                                typeof snapshot.monitor?.revision !== 'number' || !snapshot.locale?.locale) {
                                throw new Error(`Invalid mounted window: ${route} ${JSON.stringify(snapshot)}`)
                            }
                            if (process.env.ARAMGG_RELEASE_SMOKE_HIDDEN !== '1' && !window.isVisible()) throw new Error(`Window cannot be shown: ${route}`)
                            if (route === '/floating-overlay' && !snapshot.text.includes('Smoke augment')) {
                                throw new Error('Floating window did not render the diagnostic payload')
                            }
                            await window.webContents.capturePage()
                            await new Promise(resolve => setTimeout(resolve, 350))
                            const screenshot = await window.webContents.capturePage()
                            if (screenshot.isEmpty()) throw new Error(`Empty screenshot: ${route}`)
                            await fs.writeFile(path.join(outputDir, `${route.slice(1)}.png`), screenshot.toPNG())
                            snapshots.push(snapshot)
                            if (route === '/floating-overlay') {
                                // Visual-only fixtures: bundled rune art stands in for augment art offline.
                                // No network, game input, or real player data is used here.
                                await window.webContents.executeJavaScript(`(() => {
                                    const names = ['测试：超长海克斯中文标题', '测试：推荐卡片', '测试：普通卡片'];
                                    document.querySelectorAll('.augment-item').forEach((card, i) => {
                                        card.querySelector('.name').textContent = names[i];
                                        const icon = document.createElement('img');
                                        icon.src = new URL('./runes/' + [8005, 8112, 8229][i] + '.png', location.href).href;
                                        icon.style.cssText = 'width:100%;height:100%;object-fit:contain';
                                        card.querySelector('.augment-icon-frame').replaceChildren(icon);
                                    });
                                })()`)
                                await new Promise(resolve => setTimeout(resolve, 250))
                                for (const width of [760, 1000]) {
                                    window.setSize(width, 170)
                                    await window.webContents.capturePage()
                                    await new Promise(resolve => setTimeout(resolve, 150))
                                    await fs.writeFile(path.join(outputDir, `floating-chinese-${width}.png`), (await window.webContents.capturePage()).toPNG())
                                }
                                window.setSize(760, 170)
                            }
                        }
                        // Exercise the isolated app's own settings without OS input or game access.
                        await main.webContents.executeJavaScript(`document.querySelectorAll('.rail nav button')[2].click()`)
                        await new Promise((resolve) => setTimeout(resolve, 100))
                        const settings = await main.webContents.executeJavaScript(`(() => {
                            const fallback = document.querySelector('details.manual-fallback');
                            return { fallbackCollapsed: !!fallback && !fallback.open,
                                manualInputPresent: !!fallback?.querySelector('input'),
                                text: document.querySelector('main')?.textContent || '' };
                        })()`)
                        if (!settings.fallbackCollapsed || !settings.manualInputPresent) {
                            throw new Error('Manual directory must remain a collapsed advanced fallback')
                        }
                        const updateUi = await main.webContents.executeJavaScript(`(() => ({
                            panel: !!document.querySelector('.update-panel'),
                            password: document.querySelector('#update-token')?.type === 'password',
                            tokenEmpty: document.querySelector('#update-token')?.value === '',
                            overflow: document.querySelector('.update-panel')?.scrollWidth > document.querySelector('.update-panel')?.clientWidth,
                        }))()`)
                        if (!updateUi.panel || !updateUi.password || !updateUi.tokenEmpty || updateUi.overflow) {
                            throw new Error('Private update settings UI failed its packaged check')
                        }
                        await fs.writeFile(path.join(outputDir, 'update-ui.json'), JSON.stringify(updateUi, null, 2))
                        await fs.writeFile(path.join(outputDir, 'updates.png'), (await main.webContents.capturePage()).toPNG())
                        await main.webContents.executeJavaScript(`document.querySelector('details.manual-fallback').scrollIntoView({block:'center'})`)
                        await new Promise((resolve) => setTimeout(resolve, 250))
                        await fs.writeFile(path.join(outputDir, 'settings.json'), JSON.stringify(settings, null, 2))
                        await fs.writeFile(path.join(outputDir, 'settings.png'), (await main.webContents.capturePage()).toPNG())
                        // Include delayed mount effects, not just the first IPC notification.
                        if (process.env.FUZE_PIPELINE_FIXTURES) {
                            const { runPipelineSmoke } = await import('./pipeline-smoke.ts')
                            await runPipelineSmoke(outputDir)
                        }
                        await new Promise((resolve) => setTimeout(resolve, 1000))
                        if (failures.length) throw new Error(failures.join('\n'))
                    })(),
                    new Promise((_, reject) => {
                        timeout = setTimeout(() => reject(new Error('Packaged startup exceeded 45 seconds')), 45000)
                    }),
                ])
            } catch (error) {
                fail(error instanceof Error ? error.stack || error.message : String(error))
            } finally {
                clearTimeout(timeout)
                await fs.writeFile(path.join(outputDir, 'report.json'), JSON.stringify({
                    passed: failures.length === 0, packaged: app.isPackaged,
                    hidden: process.env.ARAMGG_RELEASE_SMOKE_HIDDEN === '1',
                    version: app.getVersion(), platform: process.platform,
                    durationMs: Date.now() - startedAt, readyCount: ready.size,
                    windows: snapshots, failures,
                }, null, 2))
                app.exit(failures.length ? 1 : 0)
            }
        },
    }
}
