import { build } from 'vite'
import vue from '@vitejs/plugin-vue'
import { mkdir, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { spawn } from 'node:child_process'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
const repo = fileURLToPath(new URL('..', import.meta.url))
const require = createRequire(import.meta.url)
const root = path.join(repo, 'build/partial-preview-src'), out = path.join(repo, 'build/partial-preview')
await mkdir(root, { recursive: true }); await mkdir(out, { recursive: true })
await writeFile(path.join(root, 'index.html'), '<html><head><meta charset="UTF-8"></head><body style="margin:0"><div id="app"></div><script type="module" src="./entry.ts"></script></body></html>')
await writeFile(path.join(root, 'entry.ts'), `import {createApp} from 'vue';import {createI18n} from 'vue-i18n';
import Panel from '../../src/renderer/components/AugmentFloatingOverlay.vue';import {messages} from '../../src/renderer/i18n/messages';
const listeners=new Map();window.electronAPI={events:{on:(name,fn)=>{listeners.set(name,fn);return()=>listeners.delete(name)}},diagnostics:{logRendererInfo:()=>{}},store:{get:async()=>null}} as any;
const i18n=createI18n({legacy:false,locale:'zh-CN',messages});createApp(Panel).use(i18n).mount('#app');
(window as any).fixture=(slot,locale)=>{i18n.global.locale.value=locale;listeners.get('augment-detected')({championId:1,winrateInMain:true,augments:[0,1,2].map(i=>i===slot?{id:null,missing:true,detectedSlot:i}:{id:i+1,name:['示例强化甲','示例强化乙','示例强化丙'][i],rarity:'gold',detectedSlot:i,winRate:0.52,pickRate:0.12,recommendScore:0.62})})};`)
await build({ configFile: false, root, publicDir: false, base: './', plugins: [vue()], build: { outDir: out, emptyOutDir: false } })
const runner = path.join(root, 'capture.cjs')
await writeFile(runner, `const {app,BrowserWindow,session}=require('electron');const fs=require('node:fs/promises'),path=require('node:path');
app.disableHardwareAcceleration();app.setPath('userData',${JSON.stringify(path.join(out, 'isolated-data'))});
app.whenReady().then(async()=>{session.defaultSession.webRequest.onBeforeRequest((d,cb)=>cb({cancel:d.url.startsWith('http')}));
const w=new BrowserWindow({show:false,focusable:false,width:760,height:170,useContentSize:true,webPreferences:{contextIsolation:true,sandbox:true,backgroundThrottling:false,offscreen:true}});
await w.loadFile(${JSON.stringify(path.join(out, 'index.html'))});const checks=[];
for(const locale of ['zh-CN','en-US','zh-TW'])for(const slot of [0,1,2]){await w.webContents.executeJavaScript('fixture('+slot+','+JSON.stringify(locale)+')');await new Promise(r=>setTimeout(r,350));
const check=await w.webContents.executeJavaScript("({cards:document.querySelectorAll('.augment-item').length,missing:document.querySelectorAll('.empty-slot').length,basis:document.querySelector('.statistics-basis').textContent,inside:[...document.querySelectorAll('.score-track,.missing-hint,.augment-item')].every(e=>{const r=e.getBoundingClientRect();return r.bottom<=innerHeight&&r.right<=innerWidth}),bounds:[...document.querySelectorAll('.score-track,.missing-hint,.augment-item')].map(e=>({class:e.className,bottom:e.getBoundingClientRect().bottom,right:e.getBoundingClientRect().right})),hint:document.querySelector('.missing-hint').textContent})");checks.push({locale,slot,...check});
if(locale==='zh-CN'&&slot===1)await fs.writeFile(path.join(${JSON.stringify(out)},'missing-center.png'),(await w.webContents.capturePage()).toPNG());}
await fs.writeFile(path.join(${JSON.stringify(out)},'checks.json'),JSON.stringify(checks,null,2));if(checks.some(c=>c.cards!==3||c.missing!==1||!c.inside||!c.hint||!c.basis))throw new Error('Overlay layout failed');w.destroy();app.quit();
}).catch(()=>{console.error('Isolated overlay preview failed');app.exit(1)});`)
const env = { ...process.env }; delete env.ELECTRON_RUN_AS_NODE
const child = spawn(require('electron'), [runner], { env, windowsHide: true, stdio: ['ignore', 'ignore', 'pipe'] })
child.stderr.on('data', data => { if (String(data).includes('preview failed')) process.stderr.write(data) })
const code = await new Promise(resolve => child.on('exit', resolve))
if (code !== 0) throw new Error('Overlay preview validation failed')
console.log('Isolated overlay passed: three missing positions, three locales, 760x170 content bounds')
