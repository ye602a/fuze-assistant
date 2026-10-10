import { execFile } from 'child_process'
import { desktopCapturer, type DesktopCapturerSource, type Size } from 'electron'
import logger from './modules/logger.ts'
import { bitmapToRgba } from '../shared/raw-frame.ts'
import { readCaptureStream } from './capture-stream.ts'

let lastCaptureSourceKey: string | null = null
export const CAPTURE_THUMBNAIL_SIZE = { width: 1280, height: 720 }
const DEFAULT_CAPTURE_TIMEOUT_MS = 4000
const pendingSourceRequests = new Map<string, Promise<DesktopCapturerSource[]>>()

function captureSources(types: Array<'screen' | 'window'>, thumbnailSize: Size) {
    const key = `${types.join(',')}:${thumbnailSize.width}x${thumbnailSize.height}`
    const previous = pendingSourceRequests.get(key)
    if (previous) return previous
    // A timeout cannot cancel the native call. Reuse it until it really settles.
    const request = Promise.resolve().then(() => desktopCapturer.getSources({ types, thumbnailSize, fetchWindowIcons: false }))
        .finally(() => { if (pendingSourceRequests.get(key) === request) pendingSourceRequests.delete(key) })
    pendingSourceRequests.set(key, request)
    return request
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number, label: string): Promise<T> {
    if (!timeoutMs || timeoutMs <= 0) {
        return promise
    }

    let timeoutId: NodeJS.Timeout | null = null
    const timeoutPromise = new Promise<never>((_, reject) => {
        timeoutId = setTimeout(() => {
            reject(new Error(`${label} timed out after ${timeoutMs}ms`))
        }, timeoutMs)
    })

    return Promise.race([promise, timeoutPromise]).finally(() => {
        if (timeoutId) {
            clearTimeout(timeoutId)
        }
    })
}

/**
 * 查找游戏窗口
 * @param {Array} sources - desktopCapturer 返回的源列表
 * @returns {Object|null} 匹配的游戏窗口源，未找到返回 null
 */
const findGameWindow = (sources: DesktopCapturerSource[]): DesktopCapturerSource | null => {
    const candidates = sources.map(source => {
        const name = source.name.toLowerCase()
        const isOwnWindow =
            name.includes('aramgg_client') ||
            name.includes('lol_tips_client') ||
            name.includes('champr') || name.includes('符择')

        if (isOwnWindow) {
            return { source, score: 0 }
        }

        if (name.includes('league of legends') && name.includes('client')) {
            return { source, score: 100 }
        }

        if (name.includes('league of legends') || name.includes('英雄联盟')) {
            return { source, score: 80 }
        }

        if (name.includes('league') && name.includes('client')) {
            return { source, score: 60 }
        }

        return { source, score: 0 }
    })

    candidates.sort((a, b) => b.score - a.score)
    return candidates[0]?.score > 0 ? candidates[0].source : null
}

const isLolGameProcessRunning = (): Promise<boolean | null> => {
    if (process.platform !== 'win32') {
        return Promise.resolve(null)
    }

    return new Promise<boolean | null>((resolve) => {
        execFile(
            'tasklist',
            ['/FI', 'IMAGENAME eq League of Legends.exe', '/FO', 'CSV', '/NH'],
            { timeout: 3000, windowsHide: true },
            (error, stdout = '') => {
                if (error) {
                    logger.debug('Unable to query LoL game process:', error.message)
                    resolve(null)
                    return
                }

                resolve(stdout.toLowerCase().includes('league of legends.exe'))
            }
        )
    })
}

/**
 * 查找屏幕源（回退方案）
 * @param {Array} sources - desktopCapturer 返回的源列表
 * @returns {Object|null} 主屏幕源，未找到返回 null
 */
const findScreenSource = (sources: DesktopCapturerSource[]): DesktopCapturerSource | null => {
    // 优先使用主显示器（screen:0:0 通常是主屏幕）
    const screens = sources.filter((source) => source.id.startsWith('screen:'))
    return screens.length > 0 ? screens[0] : null
}

/**
 * 尝试通过 desktopCapturer 获取 LoL 游戏窗口 ID
 * @returns {Promise<string|null>} 游戏窗口 ID，未找到返回 null
 */
export const getLolGameWindowId = async () => {
    try {
        const sources = await desktopCapturer.getSources({
            types: ['window'],
            thumbnailSize: { width: 0, height: 0 },
            fetchWindowIcons: false,
        })
        const gameWindow = findGameWindow(sources)
        return gameWindow ? gameWindow.id : null
    } catch (error) {
        logger.error('获取游戏窗口 ID 失败:', error)
        return null
    }
}

export const getLolGameWindowInfo = async () => {
    try {
        const sources = await desktopCapturer.getSources({
            types: ['window'],
            thumbnailSize: { width: 0, height: 0 },
            fetchWindowIcons: false,
        })
        const gameWindow = findGameWindow(sources)
        return {
            found: !!gameWindow,
            id: gameWindow?.id || null,
            name: gameWindow?.name || null,
        }
    } catch (error) {
        logger.error('Failed to query LoL game window:', error)
        return {
            found: false,
            id: null,
            name: null,
            error: error instanceof Error ? error.message : String(error),
        }
    }
}

async function queryLolGameStatus() {
    const processRunning = await isLolGameProcessRunning()
    const windowInfo = processRunning === false
        ? { found: false, id: null, name: null }
        : await getLolGameWindowInfo()
    return { ...windowInfo, processRunning, isGameOpen: windowInfo.found && processRunning !== false }
}
let gameStatusPromise: ReturnType<typeof queryLolGameStatus> | null = null
let gameStatusExpires = 0
export const getLolGameStatus = () => {
    if (!gameStatusPromise || Date.now() >= gameStatusExpires) {
        gameStatusExpires = Infinity
        gameStatusPromise = queryLolGameStatus().finally(() => { gameStatusExpires = Date.now() + 5000 })
    }
    return gameStatusPromise
}

/**
 * 截图当前屏幕（优先截取游戏窗口）
 * 截图直接返回内存Buffer，不进行磁盘I/O
 * @param {Object} options - 截图选项
 * @returns {Promise<Object>} 截图结果
 */
type CaptureScreenshotOptions = {
    stream?: boolean
    raw?: boolean
    preferScreen?: boolean
    timeoutMs?: number
    thumbnailSize?: Size
}

export const captureScreenshot = async (options: CaptureScreenshotOptions = {}) => {
    try {
        const {
            raw = false,
            preferScreen = false,
            timeoutMs = DEFAULT_CAPTURE_TIMEOUT_MS,
            thumbnailSize = CAPTURE_THUMBNAIL_SIZE,
        } = options || {}
        const timestamp = Date.now()
        if (raw && preferScreen && options.stream !== false) {
            const frame = await readCaptureStream()
            if (frame === 'unchanged') return { success: false, unchanged: true, error: 'unchanged-frame' }
            if (frame) return { success: true, buffer: frame, timestamp, hasLolWindow: false, captureMode: 'screen', width: frame.width, height: frame.height }
        }
        const sourceTypes: Array<'screen' | 'window'> = preferScreen
            ? ['screen']
            : ['window', 'screen']

        // 获取所有窗口和屏幕源
        const sources = await withTimeout(
            captureSources(sourceTypes, thumbnailSize),
            timeoutMs,
            `desktopCapturer.getSources(${sourceTypes.join(',')})`
        )

        // 查找游戏窗口
        const gameWindow = preferScreen ? null : findGameWindow(sources)

        // 确定截图源：优先游戏窗口，否则使用全屏
        let captureSource: DesktopCapturerSource | null = gameWindow
        let captureMode: 'window' | 'screen' = 'window'

        if (!gameWindow) {
            captureSource = findScreenSource(sources)
            captureMode = 'screen'
        }

        if (!captureSource) {
            throw new Error('无法获取截图源')
        }

        // 获取截图
        const screenshot = captureSource.thumbnail

        if (!screenshot || screenshot.isEmpty()) {
            throw new Error('截图为空')
        }

        // 转换为 PNG Buffer（内存中，不保存到文件）
        const bitmapSize = screenshot.getSize()
        const pngBuffer = raw
            ? bitmapToRgba(screenshot.toBitmap(), bitmapSize.width, bitmapSize.height)
            : screenshot.toPNG()
        const hasLolWindow = !!gameWindow
        const size = screenshot.getSize()
        const sourceName = gameWindow?.name || captureSource.name || 'screen'
        const sourceKey = `${captureMode}:${captureSource.id}`

        if (sourceKey !== lastCaptureSourceKey) {
            lastCaptureSourceKey = sourceKey
            if (preferScreen) {
                logger.info(`Screenshot source changed: screen capture "${sourceName}" (${size.width}x${size.height})`)
            } else if (gameWindow) {
                logger.info(`Screenshot source changed: game window "${gameWindow.name}" (${size.width}x${size.height})`)
            } else {
                logger.warn(`Screenshot source changed: LoL window not found, using ${captureMode} "${sourceName}" (${size.width}x${size.height})`)
            }
        }

        logger.debug(`Screenshot captured: mode=${captureMode}, size=${size.width}x${size.height}, raw=${raw}`)

        return {
            success: true,
            buffer: pngBuffer,
            timestamp,
            hasLolWindow,
            captureMode,
            windowName: gameWindow?.name,
            width: size.width,
            height: size.height
        }
    } catch (error) {
        logger.error('Screenshot capture failed:', error)
        return {
            success: false,
            error: error instanceof Error ? error.message : String(error),
        }
    }
}

// 截图模块已不再需要文件管理功能
// 所有截图直接保存在内存Buffer中进行OCR识别
// 如需调试保存截图，可在调用方自行处理
