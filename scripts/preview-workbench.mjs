import { build } from 'vite'
import vue from '@vitejs/plugin-vue'
import { mkdir, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const require = createRequire(import.meta.url)
const repo = fileURLToPath(new URL('..', import.meta.url))
const root = path.join(repo, 'build/workbench-preview-src'), out = path.join(repo, 'build/workbench-preview')
await mkdir(root, { recursive: true }); await mkdir(out, { recursive: true })
await writeFile(path.join(root, 'index.html'), '<html><head><meta charset="UTF-8"></head><body style="margin:0"><div id="app"></div><script type="module" src="./entry.ts"></script></body></html>')
await writeFile(path.join(root, 'entry.ts'), `import {createApp} from 'vue';import {createI18n} from 'vue-i18n';
import Panel from '../../src/renderer/components/Display.vue';import {messages} from '../../src/renderer/i18n/messages';
const status=()=>({sampledAt:Date.now(),version:'0.6.1',connection:{discovered:true,verifiedAt:Date.now(),phase:'ChampSelect',error:null},automatic:true,
 capture:{running:false,capturing:false,analyzing:false,count:0,analysisCount:0,lastCaptureAt:null,lastAnalysisMs:0},ocr:'ready',ocrExecution:{backend:'webgpu',warming:false,fallback:false,reason:''},data:{version:'演示数据',patch:'16.20',state:'ready'},recent:{detectedAt:null,augments:[]},resources:{sampledAt:Date.now(),cpuPercent:0.7,workingSetMB:168,windows:1},
 levelGate:{mode:'preparing',level:null,nextLevel:3,skipped:0},recognitionData:{referencePatch:'16.20',catalogPatch:'16.20',count:201,catalogDate:'演示',attribution:'Preview fixtures only',providers:[]}});
let checks=0;window.electronAPI={events:{on:()=>()=>{}},appInfo:{getRuntimeStatus:async()=>status(),setAutomatic:async()=>({success:true})},store:{get:async()=>true,set:async()=>{}},
 lcu:{getManualLeaguePath:async()=>({path:''})},appUpdate:{getState:async()=>({success:true,data:{status:'idle',currentVersion:'0.6.1',tokenConfigured:false}})},
 diagnostics:{inspectStageSignal:async()=>({report:{checkedAt:Date.now(),actualStage:null,level:11,truncated:false,schemaFields:[],playerFields:[],augmentEventCount:checks++,context:{revision:1,status:'stable'},endpoints:['schema','player','events'].map(endpoint=>({endpoint,status:'ok',httpStatus:200}))},retryAfterMs:30000})}} as any;
createApp(Panel).use(createI18n({legacy:false,locale:'zh-CN',messages})).mount('#app');`)
await build({ configFile: false, root, publicDir: path.join(repo, 'public'), base: './', plugins: [vue()], build: { outDir: out, emptyOutDir: false } })
const runner = path.join(root, 'capture.cjs')
await writeFile(runner, `const {app,BrowserWindow,session}=require('electron');const fs=require('node:fs/promises'),path=require('node:path');
app.disableHardwareAcceleration();app.setPath('userData',${JSON.stringify(path.join(out, 'isolated-data'))});
app.whenReady().then(async()=>{session.defaultSession.webRequest.onBeforeRequest((d,cb)=>cb({cancel:d.url.startsWith('http')}));
const w=new BrowserWindow({show:false,width:1180,height:920,webPreferences:{contextIsolation:true,sandbox:true,backgroundThrottling:false}});
const errors=[];w.webContents.on('console-message',(event)=>{if(/Uncaught|Unhandled error/.test(event.message))errors.push(event.message)});
await w.loadFile(${JSON.stringify(path.join(out, 'index.html'))});await new Promise(r=>setTimeout(r,300));
const checks=[];for(const width of [1180,860,600]){w.setSize(width,920);await w.webContents.executeJavaScript("document.querySelector('nav button').click();document.querySelector('main').scrollTop=0");await new Promise(r=>setTimeout(r,200));
checks.push(await w.webContents.executeJavaScript("({width:innerWidth,mainOverflow:document.querySelector('main').scrollWidth>document.querySelector('main').clientWidth,bodyOverflow:document.documentElement.scrollWidth>innerWidth,icon:document.querySelector('.brand-mark').naturalWidth>0})"));await fs.writeFile(path.join(${JSON.stringify(out)},'overview-'+width+'.png'),(await w.webContents.capturePage()).toPNG());}
w.setSize(1180,920);await w.webContents.executeJavaScript("document.querySelectorAll('nav button')[2].click()");await new Promise(r=>setTimeout(r,200));await fs.writeFile(path.join(${JSON.stringify(out)},'settings.png'),(await w.webContents.capturePage()).toPNG());
await w.webContents.executeJavaScript("document.querySelector('.stage-probe select').value='anvil';document.querySelector('.stage-probe select').dispatchEvent(new Event('change',{bubbles:true}));document.querySelector('.stage-probe button').click()");await new Promise(r=>setTimeout(r,100));
await w.webContents.executeJavaScript("document.querySelectorAll('.stage-probe button')[1].click();document.querySelector('.stage-probe button').click()");await new Promise(r=>setTimeout(r,100));
await w.webContents.executeJavaScript("document.querySelector('.stage-probe').scrollIntoView({block:'center'})");await new Promise(r=>setTimeout(r,200));await fs.writeFile(path.join(${JSON.stringify(out)},'claim-comparison.png'),(await w.webContents.capturePage()).toPNG());
const claim=await w.webContents.executeJavaScript("({unknown:document.querySelector('.stage-probe').innerText.includes('实际领取轮次：尚未确认'),comparison:document.querySelector('.comparison')!==null})");
await fs.writeFile(path.join(${JSON.stringify(out)},'checks.json'),JSON.stringify({checks,claim,errors},null,2));app.exit(checks.some(c=>c.mainOverflow||c.bodyOverflow||!c.icon)||!claim.unknown||!claim.comparison||errors.length?1:0);
}).catch(()=>{console.error('Isolated workbench preview failed');app.exit(1)});`)
const env = { ...process.env }; delete env.ELECTRON_RUN_AS_NODE
const child = spawn(require('electron'), [runner], { env, windowsHide: true, stdio: 'inherit' })
const timeout = setTimeout(() => child.kill(), 60000)
const code = await new Promise((resolve, reject) => { child.on('error', reject); child.on('exit', resolve) })
clearTimeout(timeout)
if (code !== 0) throw new Error('Workbench preview validation failed')
console.log('Isolated workbench preview passed: 1180 / 860 / 600 px, no overflow, stage unconfirmed')
