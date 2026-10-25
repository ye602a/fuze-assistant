// Rebuild the bounded item policy from saved, matching-patch public catalogues.
// Usage: node scripts/prepare-tencent-items.mjs <Tencent items.js JSON> <16.21 client items.json>
import fs from 'node:fs';
import crypto from 'node:crypto';
const [officialPath, clientPath] = process.argv.slice(2);
const officialBytes = fs.readFileSync(officialPath), clientBytes = fs.readFileSync(clientPath);
const official = JSON.parse(officialBytes), client = JSON.parse(clientBytes);
if (official.version !== '16.21' || !Array.isArray(client)) throw new Error('matching 16.21 catalogues required');
const t = new Map(official.items.map(i => [Number(i.itemId), i]));
const c = new Map(client.map(i => [i.id, i]));
// Some Mayhem variants (e.g. 126697) have an explicitly unspecified map in 101.
// They still need the client purchase flags and must occur in the Mayhem endpoint.
const mayhem = meta => meta?.maps.includes('嚎哭深渊') || meta?.maps.includes('未知');
const normal = i => i?.inStore === true && i.priceTotal > 0 && !i.requiredChampion && !i.requiredAlly
  && !i.requiredBuffCurrencyName && !i.isEnchantment && mayhem(t.get(i.id))
  && !/需要强化符文|需要海克斯/.test(t.get(i.id)?.item_desc ?? '')
  && (i.displayInItemSets || i.from.length > 0);
const data = [];
for (const i of client) {
  const base = c.get(i.specialRecipe);
  const purchase = normal(i) ? i : !i.inStore && !i.requiredChampion && !i.requiredAlly
    && !i.requiredBuffCurrencyName && normal(base) ? base : null;
  const meta = t.get(i.id) ?? (purchase && t.get(purchase.id));
  if (!mayhem(meta)) continue;
  data.push({id: i.id, name: i.name, iconUrl: meta.iconPath,
    purchaseId: purchase?.id ?? null,
    reason: purchase ? '' : /需要强化符文/.test(meta.item_desc ?? '')
      ? meta.item_desc.match(/需要强化符文[^\n]*/)[0] : '特殊或条件装备，不自动导入'});
}
data.sort((a,b) => a.id-b.id);
const result = {patch:'16.21', sources:[
  {url:'https://game.gtimg.cn/images/lol/act/img/js/items/items.js', sha256:crypto.createHash('sha256').update(officialBytes).digest('hex')},
  {url:'https://raw.communitydragon.org/16.21/plugins/rcp-be-lol-game-data/global/zh_cn/v1/items.json',sha256:crypto.createHash('sha256').update(clientBytes).digest('hex'),note:'Saved 2026-10-22 client export; current anonymous fetch returned 403.'}
],data};
fs.writeFileSync(new URL('../resources/tencent-mayhem-items-16.21.json',import.meta.url),JSON.stringify(result,null,2)+'\n');
console.log(`${data.length} mapped items; ${data.filter(i=>i.purchaseId && i.purchaseId!==i.id).length} verified recipe substitutions`);
