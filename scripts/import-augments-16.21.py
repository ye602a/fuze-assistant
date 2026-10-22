import json,sys,re,hashlib
from pathlib import Path
# Offline importer; development-only dependency: xxhash (XXH3 string-table keys).
import xxhash
import argparse
parser = argparse.ArgumentParser(description='Import fixed 16.21 Riot client metadata; no statistics.')
parser.add_argument('source_directory', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
d, out = args.source_directory, args.output
s=json.loads((d/'strings.json').read_text(encoding='utf8'))['entries'];k=json.loads((d/'kiwi.json').read_text(encoding='utf8'));zh={x['id']:x for x in json.loads((d/'cherry-augments-zh_cn.json').read_text(encoding='utf8'))}
def lookup(key):
 key=key.strip()
 return s.get(key.lower()) or s.get('{%010x}'%(xxhash.xxh3_64_intdigest(key.lower())&((1<<38)-1)))
def resolve(key,depth=0):
 v=lookup(key)
 if not v or depth>4: raise ValueError(key)
 return re.sub(r'\{\{([^{}]+)\}\}',lambda m:resolve(m[1],depth+1),v)
rows=[]
for a in k.values():
 if not isinstance(a,dict) or a.get('__type')!='AugmentData':continue
 if a['AugmentNameId'].startswith('GoH_PentakillAug_'):continue
 id=a['AugmentPlatformId']; z=zh[id]
 assert resolve(a['NameTra'])==z['nameTRA'],(id,resolve(a['NameTra']),z['nameTRA'])
 desc=resolve(a.get('DescriptionTra') or a['AugmentTooltipTra'])
 desc=re.sub(r'<[^>]*>',' ',desc);desc=re.sub(r'@[^@]+@','?',desc);desc=re.sub(r'%i:[^%]+%','',desc);desc=' '.join(desc.split())
 url='https://raw.communitydragon.org/16.21/plugins/rcp-be-lol-game-data/global/default/'+z['augmentSmallIconPath'].split('/assets/',1)[1].lower()
 rows.append(dict(id=id,key=a['AugmentNameId'],name=z['nameTRA'],rarity={'kSilver':0,'kGold':1,'kPrismatic':2}[z['rarity']],iconUrl=url,description=desc,enabled=True))
obj={'gamePatch':'16.21','locale':'zh-CN','source':'Riot client assets exported by CommunityDragon; KIWI mode only','sources':[{'url':'https://raw.communitydragon.org/16.21/'+p,'sha256':hashlib.sha256((d/f).read_bytes()).hexdigest()} for p,f in [('plugins/rcp-be-lol-game-data/global/zh_cn/v1/cherry-augments.json','cherry-augments-zh_cn.json'),('game/maps/modespecificdata/kiwi.bin.json','kiwi.json'),('game/zh_cn/data/menu/en_us/lol.stringtable.json','strings.json')]],'descriptionNote':'Client runtime values are shown as ?; no estimated values or statistics.','data':sorted(rows,key=lambda x:x['id'])}
out.write_text(json.dumps(obj,ensure_ascii=False,indent=2)+'\n',encoding='utf8');print('Imported %d ordinary KIWI augments' % len(rows))


