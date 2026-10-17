// Build-time only. Runtime downloads a bounded native archive; no shell extraction or ZIP traversal.
import {open,readFile,writeFile,stat} from 'node:fs/promises'
import {createHash} from 'node:crypto'
import {deflateRawSync,gzipSync} from 'node:zlib'
import {execFileSync} from 'node:child_process'
import path from 'node:path'
import AdmZip from 'adm-zip'
import {root} from './rust-prototype-tools.mjs'
import {updateEntries} from './rust-bundle-policy.mjs'
const git=(args)=>execFileSync('git',args,{cwd:root,encoding:'utf8',windowsHide:true}).trim()
const revision=git(['rev-parse','--short=12','HEAD'])
const sequence=Number(git(['show','-s','--format=%ct','HEAD']))
if(!/^[a-f0-9]{12}$/.test(revision)||!Number.isSafeInteger(sequence)||sequence<=0)throw new Error('Invalid release identity')
const directory=path.resolve(process.env.FUZE_RUST_PREVIEW_DIR || path.join(root,'rust/artifacts/preview'))
const zip=new AdmZip(path.join(directory,`fuze-rust-preview-${revision}.zip`))
const entries=updateEntries(zip.getEntries())
const files=[]
for(const e of entries){
 const data=e.getData()
 if(data.length!==e.header.size)throw new Error('Invalid update file size')
 const packed=deflateRawSync(data,{level:6})
 if(packed.length>128*1024*1024)throw new Error('Invalid compressed update size')
 files.push({path:e.entryName,size:data.length,compressed:packed.length,packed})
}
const index=Buffer.from(JSON.stringify({schema:1,revision,files:files.map(({packed,...f})=>f)}))
if(index.length>512*1024)throw new Error('Invalid update index size')
const header=Buffer.alloc(12);header.write('FUZEUP01');header.writeUInt32LE(index.length,8)
const archive=`Fuze-Rust-${revision}.fuz`
const handle=await open(path.join(directory,archive),'w')
const hash=createHash('sha512'),sizes=[],checksums=[]
try{
 for(const bytes of [Buffer.concat([header,index]),...files.map(f=>f.packed)]){
  hash.update(bytes)
  for(let offset=0;offset<bytes.length;offset+=1024*1024){const chunk=bytes.subarray(offset,offset+1024*1024);sizes.push(chunk.length);checksums.push(createHash('sha256').update(chunk).digest('base64'));let written=0;while(written<chunk.length){const n=await handle.write(chunk,written,chunk.length-written);if(!n.bytesWritten)throw new Error('Incomplete archive');written+=n.bytesWritten}}
 }
 await handle.sync()
}finally{await handle.close()}
const map=gzipSync(JSON.stringify({version:'2',files:[{name:'file',offset:0,sizes,checksums}]}))
await writeFile(path.join(directory,archive+'.blockmap'),map)
const manifest={schema:1,channel:'rust-preview',revision,sequence,archive,size:(await stat(path.join(directory,archive))).size,sha512:hash.digest('hex'),blockmap_sha256:createHash('sha256').update(map).digest('hex')}
await writeFile(path.join(directory,'rust-update.json'),JSON.stringify(manifest,null,2))
console.log(JSON.stringify({native_update_ready:true,revision,files:files.length,bytes:manifest.size,blocks:sizes.length,credentials:false}))
