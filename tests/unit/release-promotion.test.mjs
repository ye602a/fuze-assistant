import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {retainedDownloads} from '../../scripts/release/retained-downloads.mjs';

// Execute the actual publisher against a closed fake GitHub; unexpected I/O fails.
async function mockGitHub() {
 const {createHash}=await import('node:crypto');
 const {default:assert}=await import('node:assert/strict');
 const mode=process.env.TEST_MODE,online=!mode.startsWith('full');
 const source='a'.repeat(40),revision=source.slice(0,12),version='0.7.16';
 Object.assign(process.env,{SOURCE_SHA:source,FUZE_RELEASE_VERSION:version,FUZE_ONLINE_ONLY:String(online),FUZE_BUILD_SETUP:String(mode!=='full-no-setup')});
 const hash=(b,a='sha256')=>createHash(a).update(b).digest('hex');
 const archive=Buffer.from('update'),map=Buffer.from('blocks'),zip=Buffer.from('zip'),setup=Buffer.from('setup');
 const receipt={source,version,sequence:1,onlineOnly:online,buildRun:123,zipSha256:hash(zip),setupSha256:online||mode==='full-no-setup'?null:hash(setup),buildEvidence:{completed:true,sourceClean:true},updateValidation:{package_verified:true}};
 const manifest={revision,sequence:1,archive:`Fuze-Rust-${revision}.fuz`,size:archive.length,sha512:mode==='corrupt'?'wrong':hash(archive,'sha512'),blockmap_sha256:hash(map)};
 const files=new Map(Object.entries({[`build-evidence-${revision}.json`]:Buffer.from(JSON.stringify(receipt)), 'rust-update.json':Buffer.from(JSON.stringify(manifest)), 'rust-update-v2.json':Buffer.from(JSON.stringify({version:mode==='wrong-version'?'0.7.15':version,manifest})),[manifest.archive]:archive,[manifest.archive+'.blockmap']:map,...(!online?{[`fuze-rust-preview-${revision}.zip`]:zip,...(mode==='full'?{[`FUZE-Setup-${version}-preview-${revision}-windows-x64.exe`]:setup}:{})}:{})}));
 let id=1,release,latest,patch;
 const assets=[...files].map(([name,b])=>({id:id++,name,size:b.length,digest:`sha256:${hash(b)}`,browser_download_url:`https://files.test/${name}`}));
 const preview={id:1,prerelease:true,draft:false,assets};
 const oldAssets=[{name:'FUZE-0.7.13-windows-x64.zip',state:'uploaded',size:10,browser_download_url:'https://files.test/old.zip'},{name:'FUZE-Setup-0.7.13-windows-x64.exe',state:'uploaded',size:10,browser_download_url:'https://files.test/old.exe'}];
 if(mode==='retry') release={id:2,target_commitish:source,assets:[{...assets[0],id:20}],draft:true};
 const json=(x,status=200)=>new Response(JSON.stringify(x),{status});
 globalThis.fetch=async (url,options={})=>{
  const u=new URL(url),method=options.method||'GET',body=options.body;
  if(u.hostname==='files.test') {assert.equal(method,'GET'); assert.ok(files.has(u.pathname.slice(1)));return new Response(files.get(u.pathname.slice(1)));}
  const p=u.pathname.replace('/repos/ye602a/fuze-assistant','');
  if(method==='GET'&&p==='/releases/tags/rust-preview')return json(preview);
  if(method==='GET'&&p==='/releases'&&u.search)return json([{tag_name:'rust-v0.7.15',assets:[]},{tag_name:'rust-v0.7.13',assets:oldAssets}]);
  if(method==='GET'&&p==='/releases/tags/rust-v0.7.16')return release?json(release):json({},404);
  if(method==='POST'&&p==='/releases'){release={...JSON.parse(body),id:2,assets:[]};return json(release);}
  if(method==='POST'&&p==='/releases/2/assets'){
   const name=u.searchParams.get('name'); if(online)assert.ok(name.startsWith('build-evidence-'));
   const a={id:id++,name,size:body.length,digest:`sha256:${hash(body)}`};release.assets.push(a);return json(a);
  }
  if(method==='PATCH'&&p.startsWith('/releases/assets/')){const a=release.assets.find(a=>a.id===Number(p.split('/').pop()));assert.ok(a);Object.assign(a,JSON.parse(body));return json(a);}
  if(method==='PATCH'&&p==='/releases/2'){patch=JSON.parse(body);Object.assign(release,patch);latest=release;return json(release);}
  if(method==='GET'&&p==='/releases/latest')return json(latest);
  throw Error(`Unexpected request: ${method} ${url}`);
 };
 process.on('exit',code=>{
  if(code!==0)return;
  assert.equal(patch.name,'FUZE v0.7.16');assert.equal(patch.make_latest,'true');assert.equal(patch.draft,false);assert.equal(patch.prerelease,false);
  if(online){assert.ok(patch.body.includes('https://files.test/old.zip'));assert.ok(patch.body.includes('https://files.test/old.exe'));assert.equal(release.assets.length,1);}
  else assert.equal(release.assets.length,mode==='full'?5:3);
 });
}

for(const mode of ['online','retry','full','full-no-setup','corrupt','wrong-version'])test(`promotion: ${mode}`,()=>{
 const preload=`data:text/javascript,${encodeURIComponent(`await (${mockGitHub.toString()})();`)}`;
 const r=spawnSync(process.execPath,['--import',preload,'scripts/release/promote-stable.mjs'],{cwd:new URL('../../',import.meta.url),env:{...process.env,TEST_MODE:mode},encoding:'utf8'});
 if(['corrupt','wrong-version'].includes(mode)){assert.notEqual(r.status,0);assert.match(r.stderr,/Published online (identity|integrity) mismatch/);}
 else assert.equal(r.status,0,r.stderr);
});

test('retained downloads search beyond online releases and keep older Setup',async()=>{
 const empty={tag_name:'rust-v0.7.15',assets:[]};let calls=0;
 const mk=(version,setup)=>({tag_name:`rust-v${version}`,assets:[{name:setup?`FUZE-Setup-${version}-windows-x64.exe`:`FUZE-${version}-windows-x64.zip`,state:'uploaded',size:10,browser_download_url:version}]});
 const result=await retainedDownloads(async()=>++calls===1?Array(100).fill(empty):[mk('0.7.14',false),mk('0.7.13',true)],'api');
 assert.equal(calls,2);assert.equal(result.zip.version,'0.7.14');assert.equal(result.setup.version,'0.7.13');
 await assert.rejects(retainedDownloads(async()=>[empty],'api'),/No retained/);
});
