export interface RawFrame {
  buffer: Buffer
  width: number
  height: number
  channels: 4
}

export function isRawFrame(value: unknown): value is RawFrame {
  const frame = value as RawFrame | null
  return !!frame && Buffer.isBuffer(frame.buffer) && Number.isInteger(frame.width) &&
    Number.isInteger(frame.height) && frame.width > 0 && frame.height > 0 &&
    frame.width <= 8192 && frame.height <= 8192 && frame.channels === 4 &&
    frame.buffer.length === frame.width * frame.height * 4
}

// Electron NativeImage bitmap bytes on Windows are BGRA. Own the returned buffer.
export function bitmapToRgba(buffer: Buffer, width: number, height: number): RawFrame {
  const frame = { buffer, width, height, channels: 4 as const }
  if (!isRawFrame(frame)) throw new Error('Invalid capture bitmap dimensions')
  for (let offset = 0; offset < buffer.length; offset += 4) {
    const blue = buffer[offset]!
    buffer[offset] = buffer[offset + 2]!
    buffer[offset + 2] = blue
  }
  return frame
}
