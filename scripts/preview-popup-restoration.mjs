// Render the retained Vue components offscreen; public fixture assets only, no installed app/game IO.
import { build } from 'vite'
import vue from '@vitejs/plugin-vue'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { spawn } from 'node:child_process'
import path from 'node:path'
const repo=process.cwd(),root=path.join(repo,'build/popup-reference-src'),out=path.join(repo,'build/popup-reference')
await mkdir(root,{recursive:true});await mkdir(out,{recursive:true})
const base=path.join(repo,'resources/client-data/versions/16.20.1')
const heroes=JSON.parse(await readFile(path.join(base,'champions.json'),'utf8')).data
const augments=JSON.parse(await readFile(path.join(base,'augments.json'),'utf8')).data.slice(0,3)
const ids=[1,4,17,51,157,2,3,5,6,7,8,9,10,11,12,13,14]
const rows=ids.map(id=>heroes.find(h=>h.id===id)).filter(Boolean)
const shard=JSON.parse(await readFile(path.join(base,'champion-shards/0.json'),'utf8')).champions['4'];
const art=path.join(out,'art'),urls=[...new Set([...rows.map(h=>h.iconUrl),...augments.map(a=>a.iconUrl),...shard.augments.slice(0,20).map(a=>a.iconUrl)])].slice(0,40)
const images={};let downloaded=0
for(const url of urls){
 if(!/^https:\/\/cdn\.dtodo\.cn\/hextech\/(champion-icons|augment-icons)\/[a-zA-Z0-9_-]+\.png$/.test(url))continue
 const file=path.join(art,new URL(url).pathname);await mkdir(path.dirname(file),{recursive:true})
 let bytes;try{bytes=await readFile(file)}catch{
  const r=await fetch(url,{signal:AbortSignal.timeout(10000)});if(!r.ok)continue
  if(Number(r.headers.get('content-length'))>524288)throw new Error('Public fixture image too large')
  bytes=Buffer.from(await r.arrayBuffer());if(bytes.length>524288)throw new Error('Public fixture image too large')
  await writeFile(file,bytes);downloaded++
 }
 images[url]='data:image/png;base64,'+bytes.toString('base64')
}
await writeFile(path.join(root,'index.html'),'<html><head><meta charset="UTF-8"></head><body style="margin:0;background:#111e2e"><div id="app"></div><script type="module" src="./entry.ts"></script></body></html>')
await writeFile(path.join(root,'entry.ts'),`import {createApp,h,ref} from 'vue';import {createI18n} from 'vue-i18n';import Panel from '../../src/renderer/components/AugmentWinrateOverlay.vue';import Cards from '../../src/renderer/components/AugmentFloatingOverlay.vue';import {messages} from '../../src/renderer/i18n/messages';
const heroes=${JSON.stringify(rows)},augments=${JSON.stringify(augments)},images=${JSON.stringify(images)};const listeners=new Map();const view=ref('hero');
const candidates=heroes.map((h,i)=>({championId:h.id,name:h.name,source:i<5?'teammate':'bench',iconUrl:images[h.iconUrl],winRate:h.stats.winRate,pickRate:h.stats.pickRate}));const bench={refreshable:false,status:'ready',gameflowPhase:'ChampSelect',currentChampion:candidates[1],team:candidates.slice(0,5).map((champion,slot)=>({slot,isCurrent:slot===1,champion})),candidates};
const rates=Object.fromEntries(augments.map((a,i)=>[a.id,{winRate:.52+i*.01,pickRate:.12,recommendScore:.62+i*.1}]));
window.electronAPI={events:{on:(n,fn)=>{listeners.set(n,fn);return()=>listeners.delete(n)}},diagnostics:{logRendererInfo:()=>{}},locale:{get:async()=>({locale:'zh-CN'})},store:{get:async()=>false},windows:{ready:()=>{},hidePopup:()=>{},hideAugmentSidePanel:()=>{}},winrate:{loadChampionData:async()=>({success:true,data:{stats:{winRate:.5605,pickRate:.1163,tier:1},augments,augmentStats:rates,builds:[],items:[],championName:{nameCN:'崔斯特'},championLinks:{}}})}} as any;
createApp({setup:()=>()=>view.value==='hero'?h(Panel):h(Cards)}).use(createI18n({legacy:false,locale:'zh-CN',messages})).mount('#app');
(window as any).fixture=async(kind)=>{view.value=kind==='hero'?'hero':'cards';await new Promise(r=>setTimeout(r,30));if(kind==='hero')listeners.get('for-popup')?.({championId:4,championName:'崔斯特',champSelect:true,benchRecommendation:bench,augments:[]});else listeners.get('augment-detected')?.({championId:4,winrateInMain:true,augments:augments.map((a,i)=>kind==='partial'&&i===1?{id:null,missing:true,detectedSlot:i}:{...a,iconPath:images[a.iconUrl],rarity:'gold',detectedSlot:i,...rates[a.id]})});};`)
await build({configFile:false,root,publicDir:path.join(repo,'public'),base:'./',plugins:[vue()],build:{outDir:out,emptyOutDir:false}})
const runner=path.join(root,'capture.cjs')
await writeFile(runner,`const {app,BrowserWindow,session}=require('electron');const fs=require('node:fs/promises');app.disableHardwareAcceleration();app.setPath('userData',${JSON.stringify(path.join(out,'isolated-data'))});app.whenReady().then(async()=>{session.defaultSession.webRequest.onBeforeRequest((d,cb)=>cb({cancel:d.url.startsWith('http')}));const w=new BrowserWindow({show:false,focusable:false,width:447,height:741,useContentSize:true,webPreferences:{sandbox:true,contextIsolation:true,offscreen:true,backgroundThrottling:false}});await w.loadFile(${JSON.stringify(path.join(out,'index.html'))});for(const kind of ['hero','cards','partial']){w.setContentSize(kind==='hero'?447:760,kind==='hero'?741:170);await w.webContents.executeJavaScript('fixture('+JSON.stringify(kind)+')');await new Promise(r=>setTimeout(r,350));const check=await w.webContents.executeJavaScript("({overflow:document.documentElement.scrollWidth>innerWidth,team:document.querySelectorAll('.team-member').length,cards:document.querySelectorAll('.augment-item').length,text:document.body.innerText.includes('undefined')})");if(check.overflow||check.text||(kind==='hero'?check.team!==5:check.cards!==3))throw new Error('Reference layout failed '+JSON.stringify(check));await fs.writeFile(${JSON.stringify(out)}+'/'+kind+'.png',(await w.webContents.capturePage()).toPNG());}w.destroy();app.quit();}).catch(e=>{console.error('Reference render failed: '+e.message);app.exit(1)});`)
const env={...process.env};delete env.ELECTRON_RUN_AS_NODE
const child=spawn(createRequire(import.meta.url)('electron'),[runner],{env,windowsHide:true,stdio:['ignore','ignore','pipe']})
const timer=setTimeout(()=>child.kill(),60000)
child.stderr.on('data',data=>{if(String(data).includes('Reference render failed'))process.stderr.write(data)})
const code=await new Promise((resolve,reject)=>{child.on('exit',resolve);child.on('error',reject)});clearTimeout(timer)
if(code!==0)throw new Error('Reference render failed')
const snapshot=JSON.parse(await readFile('rust/artifacts/snapshot.json','utf8'))
snapshot.slots=augments.map(a=>({state:'known',id:a.id,name:a.name,confidence:1}))
await writeFile(path.join(out,'snapshot.json'),JSON.stringify(snapshot))
await writeFile(path.join(out,'overlay-review.json'),JSON.stringify(augments.map((a,i)=>({id:a.id,win:.52+i*.01,pick:.12,score:.62+i*.1,...(images[a.iconUrl]?{image:path.join(art,new URL(a.iconUrl).pathname)}:{})}))))
console.log(JSON.stringify({reference:'retained-vue-components',hidden:true,game_access:false,publicImages:Object.keys(images).length,downloaded,art}))
