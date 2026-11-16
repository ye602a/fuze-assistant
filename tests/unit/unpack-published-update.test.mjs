import test from 'node:test';
import assert from 'node:assert/strict';
import {deflateRawSync} from 'node:zlib';
import {unpackPublishedUpdate} from '../../scripts/release/unpack-published-update.mjs';
const revision='a'.repeat(12),data=Buffer.from('player'),packed=deflateRawSync(data);
function archive(paths=['fuze-ui-lab.exe'],extra={}){
 const index=Buffer.from(JSON.stringify({schema:1,revision,files:paths.map(path=>({path,size:data.length,compressed:packed.length,...extra}))}));
 const header=Buffer.alloc(12);header.write('FUZEUP01');header.writeUInt32LE(index.length,8);
 return Buffer.concat([header,index,...paths.map(()=>packed)]);
}
test('restores player executable name without changing bytes',()=>{const entries=unpackPublishedUpdate(archive(),revision);assert.equal(entries[0].name,'FUZE.exe');assert.deepEqual(entries[0].data,data);});
test('rejects traversal, shell files and mapped-name collisions',()=>{for(const names of [['../FUZE.exe'],['Start-preview.cmd'],['fuze-ui-lab.exe','FUZE.exe']])assert.throws(()=>unpackPublishedUpdate(archive(names),revision),/path/);});
test('rejects wrong identity, truncated data and incorrect size',()=>{assert.throws(()=>unpackPublishedUpdate(archive(),'b'.repeat(12)),/identity/);assert.throws(()=>unpackPublishedUpdate(archive().subarray(0,-1),revision),/size/);assert.throws(()=>unpackPublishedUpdate(archive(undefined,{size:99}),revision),/size/);assert.throws(()=>unpackPublishedUpdate(Buffer.concat([archive(),Buffer.from('x')]),revision),/tail/);});
