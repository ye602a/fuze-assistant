// Copy only the transitive VC++ import closure; never loads a DLL.
import {readFile, copyFile, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {inspectPe, nativeFiles} from './rust-release-readiness.mjs'
import {crtNames, sha256} from './rust-runtime-contract.mjs'
const [bundle, source] = process.argv.slice(2)
const build = JSON.parse(await readFile(path.join(bundle, 'build.json'), 'utf8'))
const queue = nativeFiles.map(name => name === 'fuze-ui-lab.exe' && build.desktopExecutable === 'FUZE.exe' ? 'FUZE.exe' : name), seen = new Set(), hashes = {}
while (queue.length) {
  const name = queue.shift()
  if (seen.has(name)) continue
  seen.add(name)
  const runtime = crtNames.includes(name)
  const file = path.join(runtime ? source : bundle, name)
  const bytes = await readFile(file)
  const pe = inspectPe(bytes)
  if (pe.architecture !== 'x64') throw Error(`Expected x64: ${name}`)
  if (runtime) {
    await copyFile(file, path.join(bundle, name))
    hashes[name] = sha256(bytes)
  }
  for (const dependency of [...pe.imports, ...pe.delayImports]) {
    if (/^(msvcp|vcruntime|concrt)\d/.test(dependency)) {
      if (!crtNames.includes(dependency)) throw Error(`Unknown CRT import: ${dependency}`)
      queue.push(dependency)
    }
  }
}
if (!Object.keys(hashes).length) throw Error('No CRT imports found')
await writeFile(path.join(bundle, 'vc-runtime-files.json'), JSON.stringify(hashes,null,2))
console.log(JSON.stringify({appLocalRuntime:hashes}))
