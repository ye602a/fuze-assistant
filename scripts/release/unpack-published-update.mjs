import {inflateRawSync} from 'node:zlib';
import {managedPath} from '../rust-bundle-policy.mjs';

export function unpackPublishedUpdate(bytes, revision) {
 if(bytes.length<12||bytes.subarray(0,8).toString()!=='FUZEUP01')throw Error('Invalid archive header');
 const length=bytes.readUInt32LE(8);
 if(length>512*1024||12+length>bytes.length)throw Error('Invalid archive index');
 const index=JSON.parse(bytes.subarray(12,12+length));
 if(index.schema!==1||index.revision!==revision||!Array.isArray(index.files)||!index.files.length||index.files.length>1024)throw Error('Invalid archive identity');
 const entries=[],names=new Set();let offset=12+length,total=0;
 for(const file of index.files){
  const name=file.path==='fuze-ui-lab.exe'?'FUZE.exe':file.path;
  if(!managedPath(name)||/\.(cmd|bat|ps1)$/i.test(name)||names.has(name.toLowerCase()))throw Error('Invalid archive path');
  names.add(name.toLowerCase());total+=file.size;
  if(!Number.isSafeInteger(file.size)||file.size<0||file.size>128*1024*1024||total>512*1024*1024||!Number.isSafeInteger(file.compressed)||file.compressed<=0||offset+file.compressed>bytes.length)throw Error('Invalid archive size');
  const data=inflateRawSync(bytes.subarray(offset,offset+file.compressed),{maxOutputLength:Math.max(1,file.size)});
  if(data.length!==file.size)throw Error('Expanded size mismatch');
  entries.push({name,data});offset+=file.compressed;
 }
 if(offset!==bytes.length)throw Error('Unexpected archive tail');
 return entries;
}
