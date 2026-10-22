// Synthetic, reproducible UI-host data. Never reads installed data, accounts or the network.
import {mkdir, readFile, writeFile} from 'node:fs/promises'
import path from 'node:path'
const root = path.resolve(process.argv[2] || 'rust/ui-lab/artifacts/host-data')
const version = '16.21.1'
const prefix = path.join(root, 'versions', version)
const common = {schemaVersion: 3, locale: 'zh-CN', dataVersion: version}
const stats = {winRate: 0.52, pickRate: 0.12, games: 100, wins: 52, source: 'offline-fixture', region: 'CN', gamePatch: '16.21', tier: 2}
const heroes = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 17, 21, 51, 157].map(id => ({id, name: `样例英雄 ${id}`, title: '隔离验证', iconUrl: `https://cdn.dtodo.cn/hextech/champion-icons/${id}.png`, stats}))
// Use real current-pool IDs: obsolete synthetic IDs are intentionally filtered by production.
const metadata = JSON.parse(await readFile(new URL('../resources/augments-16.21-zh-CN.json', import.meta.url), 'utf8')).data
const augments = [3051, 1002, 1004].map((id, i) => ({...metadata.find(a => a.id === id), description: '固定测试说明，不代表实际游戏效果。', stats: {...stats, rank: i + 1}}))
const documents = {
  'augments.json': {...common, data: augments},
  'champions.json': {...common, data: heroes},
  'items.json': {...common, data: [{id: 1001, name: '测试鞋子', iconUrl: 'https://cdn.dtodo.cn/hextech/item-icons/1001.png'}]},
  'champion-shards/index.json': {...common, shards: [{path: 'champion-shards/fixture.json', championIds: heroes.map(h => h.id)}]},
  'champion-shards/fixture.json': {...common, champions: Object.fromEntries(heroes.map(h => [h.id, {champion: h, augments, builds: [{queueId: 2400, patch: '16.21', stats, startingItems: [{itemIds: [1001], ...stats}], skillOrders: [{skillKeys: ['Q', 'W', 'E', 'Q', 'Q', 'R'], ...stats}]}]}]))}
}
await mkdir(path.join(prefix, 'champion-shards'), {recursive: true})
for (const [name, document] of Object.entries(documents)) await writeFile(path.join(prefix, name), JSON.stringify(document))
await writeFile(path.join(prefix, 'manifest.json'), JSON.stringify({...common, files: Object.keys(documents).map(p => ({path: p}))}))
await writeFile(path.join(root, 'current.json'), JSON.stringify(common))
// Explicit smoke config: dummy worker paths are never started in offline mode.
await writeFile(path.join(root, 'preview.json'), JSON.stringify({bundled_data: '.', cache_data: null, locale: 'zh-CN', worker: 'unused-worker.exe', ort: 'unused-ort.dll', model: 'unused-model.onnx', dictionary: 'unused-dictionary.txt', backend: 'cpu'}))
console.log('Created synthetic UI-host fixtures; no network or personal data.')
