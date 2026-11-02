import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp,readFile,rm} from 'node:fs/promises'
import {tmpdir} from 'node:os'
import path from 'node:path'
import {execFileSync} from 'node:child_process'
import {fileURLToPath} from 'node:url'
import AdmZip from 'adm-zip'

test('update packager emits one description and an unchanged legacy manifest', async () => {
 const root=fileURLToPath(new URL('../../',import.meta.url))
 const directory=await mkdtemp(path.join(tmpdir(),'fuze-update-description-'))
 try {
  const revision=execFileSync('git',['rev-parse','--short=12','HEAD'],{cwd:root,encoding:'utf8',windowsHide:true}).trim()
  const zip=new AdmZip()
  zip.addFile('build.json',Buffer.from(JSON.stringify({revision,version:'0.7.99'})))
  zip.writeZip(path.join(directory,`fuze-rust-preview-${revision}.zip`))
  execFileSync(process.execPath,['scripts/package-rust-update.mjs'],{cwd:root,env:{...process.env,FUZE_RUST_PREVIEW_DIR:directory},windowsHide:true})
  const legacy=JSON.parse(await readFile(path.join(directory,'rust-update.json'),'utf8'))
  const described=JSON.parse(await readFile(path.join(directory,'rust-update-v2.json'),'utf8'))
  assert.deepEqual(Object.keys(legacy).sort(),['schema','channel','revision','sequence','archive','size','sha512','blockmap_sha256'].sort())
  assert.deepEqual(described.manifest,legacy)
  assert.equal(described.version,'0.7.99')
  assert.equal(described.notes,await readFile(path.join(root,'docs/RELEASE_NOTES.md'),'utf8'))
 } finally {
  if(path.dirname(directory)!==path.resolve(tmpdir())||!path.basename(directory).startsWith('fuze-update-description-'))throw Error('Unexpected fixture directory')
  await rm(directory,{recursive:true,force:true})
 }
})
