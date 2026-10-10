// Rebuild application assets while preserving the installed, ABI-compatible native dependencies.
// Usage: node scripts/package-local.mjs <base app.asar> <output app.asar>
import fs from 'node:fs'
import path from 'node:path'
import crypto from 'node:crypto'
import { fileURLToPath } from 'node:url'
import { extractFile, listPackage } from '@electron/asar'
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const [basePath, outputPath] = process.argv.slice(2).map(value => path.resolve(value))
if (!basePath || !outputPath || basePath === outputPath) throw new Error('Use distinct base and output ASAR paths')
const base = fs.readFileSync(basePath)
const dataOffset = 8 + base.readUInt32LE(4)
const header = JSON.parse(base.subarray(16, 16 + base.readUInt32LE(12)).toString('utf8'))
const packageInfo = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'))
const oldPackage = JSON.parse(extractFile(basePath, 'package.json').toString('utf8'))
if (JSON.stringify(packageInfo.dependencies) !== JSON.stringify(oldPackage.dependencies)) throw new Error('Dependency versions differ: use a full package build')
const hash = bytes => crypto.createHash('sha256').update(bytes).digest('hex')
const newBodies = new Map()
for (const directory of ['dist', 'dist-electron']) {
  const source = path.join(root, directory)
  if (!fs.statSync(source).isDirectory()) throw new Error('Run npm run build first')
  const walk = (dir, relative) => {
    const tree = { files: {} }
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const name = `${relative}/${entry.name}`
      if (entry.isDirectory()) tree.files[entry.name] = walk(path.join(dir, entry.name), name)
      else {
        const body = fs.readFileSync(path.join(dir, entry.name))
        tree.files[entry.name] = { size: body.length }
        newBodies.set(name, body)
      }
    }
    return tree
  }
  header.files[directory] = walk(source, directory)
}
for (const name of ['package.json', 'THANKS.md']) {
  const body = fs.readFileSync(path.join(root, name))
  header.files[name] = { size: body.length }
  newBodies.set(name, body)
}
let offset = 0
const bodies = []
function rebuild(tree, prefix = '') {
  for (const [name, entry] of Object.entries(tree.files || {})) {
    const relative = prefix ? `${prefix}/${name}` : name
    if (entry.files) rebuild(entry, relative)
    else if (!entry.unpacked && !entry.link) {
      const body = newBodies.get(relative) || base.subarray(dataOffset + Number(entry.offset), dataOffset + Number(entry.offset) + entry.size)
      entry.offset = String(offset)
      entry.size = body.length
      if (newBodies.has(relative)) {
        const blockSize = 4194304
        const blocks = []
        for (let i = 0; i < body.length; i += blockSize) blocks.push(hash(body.subarray(i, i + blockSize)))
        entry.integrity = { algorithm: 'SHA256', hash: hash(body), blockSize, blocks }
      }
      bodies.push(body)
      offset += body.length
    }
  }
}
rebuild(header)
const json = Buffer.from(JSON.stringify(header))
const headerPickle = Buffer.alloc(8 + Math.ceil(json.length / 4) * 4)
headerPickle.writeUInt32LE(headerPickle.length - 4, 0)
headerPickle.writeUInt32LE(json.length, 4)
json.copy(headerPickle, 8)
const sizePickle = Buffer.alloc(8)
sizePickle.writeUInt32LE(4, 0)
sizePickle.writeUInt32LE(headerPickle.length, 4)
fs.mkdirSync(path.dirname(outputPath), { recursive: true })
fs.writeFileSync(outputPath, Buffer.concat([sizePickle, headerPickle, ...bodies]))
for (const [relative, expected] of newBodies) {
  if (!extractFile(outputPath, path.normalize(relative)).equals(expected)) throw new Error(`ASAR roundtrip failed: ${relative}`)
}
const manifest = { version: packageInfo.version, createdAt: new Date().toISOString(), baseSha256: hash(base), sha256: hash(fs.readFileSync(outputPath)), changedAssets: [...newBodies.keys()], nativeDependencies: 'Preserved byte-for-byte from base installation; app.asar.unpacked unchanged', files: listPackage(outputPath).length }
fs.writeFileSync(path.join(path.dirname(outputPath), 'package-manifest.json'), JSON.stringify(manifest, null, 2))
console.log(JSON.stringify({ version: manifest.version, assetsVerified: newBodies.size, sha256: manifest.sha256 }))
