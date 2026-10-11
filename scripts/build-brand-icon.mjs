import sharp from 'sharp'
import { readFile, writeFile } from 'node:fs/promises'

// One editable vector source for the main window, tray and Windows installer.
const source = await readFile(new URL('../public/fuze.svg', import.meta.url))
const sizes = [16, 24, 32, 48, 64, 128, 256]
const images = await Promise.all(sizes.map(size => sharp(source).resize(size, size).png().toBuffer()))
const header = Buffer.alloc(6 + images.length * 16)
header.writeUInt16LE(1, 2)
header.writeUInt16LE(images.length, 4)
let offset = header.length
for (const [index, image] of images.entries()) {
  const at = 6 + index * 16
  header[at] = header[at + 1] = sizes[index] === 256 ? 0 : sizes[index]
  header.writeUInt16LE(1, at + 4)
  header.writeUInt16LE(32, at + 6)
  header.writeUInt32LE(image.length, at + 8)
  header.writeUInt32LE(offset, at + 12)
  offset += image.length
}
await writeFile(new URL('../public/fuze.ico', import.meta.url), Buffer.concat([header, ...images]))
await writeFile(new URL('../public/fuze.png', import.meta.url), images.at(-1))
console.log('Fuze icon generated: seven Windows sizes and tray PNG')
