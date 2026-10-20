import {readdir, lstat} from 'node:fs/promises'
import path from 'node:path'
import {fileURLToPath} from 'node:url'
import {previewPath} from './rust-bundle-policy.mjs'

export async function checkSetupInput(root) {
 const seen = new Set()
 let size = 0
 async function visit(dir, prefix = '') {
  if ((await lstat(dir)).isSymbolicLink()) throw Error('setup-input-link')
  for (const name of await readdir(dir)) {
   const relative = prefix + name, file = path.join(dir, name), info = await lstat(file)
   if (info.isSymbolicLink()) throw Error('setup-input-link')
   if (info.isDirectory()) { await visit(file, relative + '/'); continue }
   if (!info.isFile() || !previewPath(relative) || seen.has(relative.toLowerCase())) throw Error('setup-input-path')
   seen.add(relative.toLowerCase())
   size += info.size
   if (info.size > 128 * 1024 * 1024 || size > 512 * 1024 * 1024 || seen.size > 1024) throw Error('setup-input-size')
  }
 }
 await visit(root)
 for (const name of ['build.json','preview.json','Install-local.ps1','fuze-update.exe','fuze-ui-lab.exe'])
  if (!seen.has(name.toLowerCase())) throw Error('setup-input-incomplete')
 return {files: seen.size, bytes: size}
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url))
 console.log(JSON.stringify(await checkSetupInput(path.resolve(process.argv[2]))))
