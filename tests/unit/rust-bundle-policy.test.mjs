import test from 'node:test'
import assert from 'node:assert/strict'
import {managedFiles,managedPath,previewPath,updateEntries} from '../../scripts/rust-bundle-policy.mjs'

const entry = entryName => ({entryName,isDirectory:false,header:{size:8}})
test('portable exact-build launchers stay in ZIP but never reach managed updates',()=>{
 const names=[...managedFiles,'resources/inference.onnx','resources/dictionary.json','resources/client-data/current.json','resources/client-data/versions/zh-TW/16.20.1/augments.json']
 const direct=['Start-this-build.cmd','Start-this-build.ps1']
 for(const name of names)assert.equal(managedPath(name),true,name)
 for(const name of direct){assert.equal(previewPath(name),true);assert.equal(managedPath(name),false)}
 const result=updateEntries([...names,...direct].map(entry))
 assert.deepEqual(new Set(result.map(e=>e.entryName)),new Set(names))
 assert.equal(new Set(managedFiles).size,managedFiles.length)
})
test('update selection rejects unknown files rather than silently shipping personal data',()=>{
 for(const name of ['preferences.json','observations-v2.json','logs/runtime.json','Start-this-build-extra.ps1','resources/client-data/../preferences.json','resources/client-data/CON.json','resources/client-data/x:stream.json','resources\\inference.onnx','/build.json','resources/client-data/x./data.json']) {
  assert.equal(previewPath(name),false,name)
  assert.throws(()=>updateEntries([entry('build.json'),entry(name)]),/Invalid/)
 }
})
test('duplicate names and oversized indexes fail before archive writing',()=>{
 assert.throws(()=>updateEntries([entry('build.json'),entry('build.json')]),/duplicate/)
 assert.throws(()=>updateEntries([entry('resources/client-data/a.json'),entry('resources/client-data/A.json')]),/duplicate/)
 assert.throws(()=>updateEntries([entry('Start-this-build.cmd')]),/file count/)
 assert.throws(()=>updateEntries(Array.from({length:1025},(_,i)=>entry(`resources/client-data/${i}.json`))),/file count/)
})
test('oversized expanded payloads are rejected before decompression',()=>{
 const oversized=entry('resources/inference.onnx'); oversized.header.size=128*1024*1024+1
 assert.throws(()=>updateEntries([oversized]),/file size/)
 const large=Array.from({length:5},(_,i)=>({...entry(`resources/client-data/${i}.json`),header:{size:128*1024*1024}}))
 assert.throws(()=>updateEntries(large),/expanded size/)
})
