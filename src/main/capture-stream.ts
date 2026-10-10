import { app, BrowserWindow, desktopCapturer, screen } from 'electron'
import path from 'node:path'
import { pathToFileURL } from 'node:url'
import { isRawFrame, type RawFrame } from '../shared/raw-frame.ts'
import logger from './modules/logger.ts'

let window: BrowserWindow | null = null
let opening: Promise<void> | null = null
let generation = 0
let failedUntil = 0
let avoidedTransferBytes = 0
let duplicates = 0
let frames = 0
let lastError: string | null = null
let quitHook = false

export function getCaptureStreamStatus() {
  return { backend: window ? 'stream' : failedUntil > Date.now() ? 'bitmap-fallback' : 'idle', frames, duplicates, avoidedTransferBytes, error: lastError }
}

function deadline<T>(task: Promise<T>, ms: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout>
  return Promise.race([task, new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error('Capture stream timeout')), ms) })]).finally(() => clearTimeout(timer))
}

export function stopCaptureStream() {
  generation++
  const previous = window
  window = null
  opening = null
  if (previous && !previous.isDestroyed()) previous.destroy()
}

async function openStream() {
  if (!quitHook) { app.once('will-quit', stopCaptureStream); quitHook = true }
  const run = generation
  // Enumerate once per session; do not generate thumbnails for every monitor.
  const sources = await desktopCapturer.getSources({ types: ['screen'], thumbnailSize: { width: 0, height: 0 }, fetchWindowIcons: false })
  if (run !== generation) throw new Error('Capture cancelled')
  const primary = String(screen.getPrimaryDisplay().id)
  const source = sources.find(item => item.display_id === primary) || sources[0]
  if (!source) throw new Error('No display capture source')
  const target = path.join(app.getAppPath(), app.isPackaged ? 'dist' : 'public', 'capture-worker.html')
  const targetUrl = pathToFileURL(target).href
  const owner = new BrowserWindow({ show: false, focusable: false, skipTaskbar: true, width: 32, height: 32,
    webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false, webSecurity: true, backgroundThrottling: false, partition: 'fuze-capture' } })
  window = owner
  owner.webContents.setWindowOpenHandler(() => ({ action: 'deny' }))
  owner.webContents.on('will-navigate', event => event.preventDefault())
  owner.webContents.session.setPermissionCheckHandler((contents, permission) =>
    !owner.isDestroyed() && contents === owner.webContents && contents.getURL() === targetUrl && String(permission) === 'display-capture')
  owner.webContents.session.setPermissionRequestHandler((contents, permission, callback, details) =>
    callback(!owner.isDestroyed() && contents === owner.webContents && contents.getURL() === targetUrl &&
      details.isMainFrame && (permission === 'display-capture' ||
        (permission === 'media' && 'mediaTypes' in details && details.mediaTypes?.length === 0)) &&
      !('mediaTypes' in details && details.mediaTypes?.includes('audio'))))
  owner.webContents.session.setDisplayMediaRequestHandler((request, callback) => {
    if (owner.isDestroyed() || request.frame !== owner.webContents.mainFrame || request.frame.url !== targetUrl || request.audioRequested) { callback({}); return }
    callback({ video: source })
  })
  await owner.loadFile(target)
  if (run !== generation || owner.isDestroyed()) throw new Error('Capture cancelled')
  await owner.webContents.executeJavaScript('globalThis.fuzeCapture.start()', true)
  lastError = null
  logger.info('Persistent capture session started', { width: 1024, height: 576, maxFps: 2 })
}

export async function readCaptureStream(): Promise<RawFrame | 'unchanged' | null> {
  if (Date.now() < failedUntil) return null
  const run = generation
  try {
    if (!opening) opening = deadline(openStream(), 8000)
    await opening
    const owner = window
    if (!owner || owner.isDestroyed()) throw new Error('Capture surface unavailable')
    const result = await deadline(owner.webContents.executeJavaScript('globalThis.fuzeCapture.read()'), 2000)
    if (run !== generation) return 'unchanged'
    if (result.pending || result.unchanged) {
      duplicates++
      avoidedTransferBytes += Math.max(0, Number(result.duplicateBytes) || 0)
      return 'unchanged'
    }
    const frame = { buffer: Buffer.from(result.pixels), width: result.width, height: result.height, channels: 4 as const }
    if (!isRawFrame(frame)) throw new Error('Invalid stream frame')
    let visible = false
    for (let offset = 0; offset < frame.buffer.length; offset += 400) {
      if (frame.buffer[offset]! + frame.buffer[offset + 1]! + frame.buffer[offset + 2]! > 24) { visible = true; break }
    }
    if (!visible) throw new Error('Capture stream returned a black frame')
    frames++
    return frame
  } catch (error) {
    if (run !== generation) return 'unchanged'
    lastError = String((error as Error)?.message || error || 'Capture unavailable').slice(0, 200)
    stopCaptureStream()
    failedUntil = Date.now() + 60000
    logger.warn('Persistent capture unavailable; using bitmap fallback', { reason: lastError })
    return null
  }
}
