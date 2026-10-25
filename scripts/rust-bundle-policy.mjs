// Shared with the native installer. Only packaging/imports use this module.
import {readFileSync} from 'node:fs'

export const managedFiles = Object.freeze(readFileSync(new URL('../rust/bundle-files.txt',import.meta.url),'utf8').trim().split(/\r?\n/))
const managed = new Set(managedFiles)
const portable = new Set(['Start-this-build.cmd','Start-this-build.ps1','Install-local.ps1'])

export function managedPath(name) {
 if(typeof name!=='string' || !name || name.length>240 || name.includes('\\'))return false
 if(name.split('/').some(part=>!part || part==='.' || part==='..' || part.endsWith('.') || part.startsWith('.') || !/^[a-zA-Z0-9._-]+$/.test(part) || ['con','prn','aux','nul','com1','lpt1'].includes(part.split('.')[0].toLowerCase())))return false
 return managed.has(name) || ['resources/inference.onnx','resources/dictionary.json'].includes(name) || name.startsWith('resources/client-data/') && name.endsWith('.json')
}
export function previewPath(name) { return portable.has(name) || managedPath(name) }

export function updateEntries(entries) {
 const names=new Set(), result=[]
 let total=0
 for(const entry of entries) {
  if(entry.isDirectory)continue
  const name=entry.entryName, key=typeof name==='string' ? name.toLowerCase() : ''
  if(!previewPath(name) || names.has(key))throw new Error('Invalid or duplicate update package path')
  names.add(key)
  // ZIP launchers/bootstrap stay outside the existing native managed-file whitelist.
  if(!portable.has(name)) {
   const size=entry.header?.size
   if(!Number.isSafeInteger(size) || size<0 || size>128*1024*1024)throw new Error('Invalid update package file size')
   total+=size
   if(total>512*1024*1024)throw new Error('Invalid update package expanded size')
   result.push(entry)
  }
 }
 if(!result.length || result.length>1024)throw new Error('Invalid update package file count')
 return result.sort((a,b)=>a.entryName.localeCompare(b.entryName,'en'))
}

// Old packages remain readable; new player bundles contain no shell entry points.
export function playerPath(name) { return previewPath(name) && !/\.(cmd|bat|ps1)$/i.test(name) }
