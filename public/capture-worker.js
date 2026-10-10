// Hidden, sandboxed capture surface. No Node access, preload, account data or network.
(() => {
  const video = document.createElement('video')
  video.muted = true
  const canvas = document.createElement('canvas')
  const context = canvas.getContext('2d', { willReadFrequently: true })
  let stream, frame = 0, readFrame = -1, lastFrameAt = 0
  const onFrame = () => { frame++; lastFrameAt = performance.now(); video.requestVideoFrameCallback(onFrame) }
  globalThis.fuzeCapture = {
    async start() {
      stream = await navigator.mediaDevices.getDisplayMedia({
        audio: false, video: { width: { ideal: 1024, max: 1024 }, height: { ideal: 576, max: 576 }, frameRate: { ideal: 2, max: 2 } },
      })
      video.srcObject = stream
      lastFrameAt = performance.now()
      video.requestVideoFrameCallback(onFrame)
      await video.play()
      return true
    },
    read() {
      if (!stream?.active) throw new Error('Capture stream ended')
      if (performance.now() - lastFrameAt > 6000) throw new Error('Capture stream stalled')
      if (video.readyState < 2 || !frame) return { pending: true }
      if (readFrame === frame) return { unchanged: true }
      const scale = Math.min(1024 / video.videoWidth, 576 / video.videoHeight, 1)
      const width = Math.max(1, Math.round(video.videoWidth * scale))
      const height = Math.max(1, Math.round(video.videoHeight * scale))
      if (canvas.width !== width || canvas.height !== height) { canvas.width = width; canvas.height = height }
      context.drawImage(video, 0, 0, width, height)
      readFrame = frame
      return { width, height, channels: 4, pixels: context.getImageData(0, 0, width, height).data.buffer }
    },
    stop() { stream?.getTracks().forEach(track => track.stop()); video.srcObject = null },
  }
})()
