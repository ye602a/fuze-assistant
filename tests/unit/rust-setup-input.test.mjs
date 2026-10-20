import test from 'node:test'
import assert from 'node:assert/strict'
import {mkdtemp, writeFile, mkdir} from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import {checkSetupInput} from '../../scripts/check-rust-setup-input.mjs'

test('installer refuses private, unexpected and incomplete recursive payloads', async () => {
 const root = await mkdtemp(path.join(os.tmpdir(), 'fuze-setup-input-'))
 await assert.rejects(checkSetupInput(root), /incomplete/)
 for (const name of ['build.json','preview.json','Install-local.ps1','fuze-update.exe','fuze-ui-lab.exe'])
  await writeFile(path.join(root, name), 'fixture-not-executable')
 assert.equal((await checkSetupInput(root)).files, 5)
 await mkdir(path.join(root, 'personal'))
 await writeFile(path.join(root, 'personal', 'account.json'), 'synthetic-private-fixture')
 await assert.rejects(checkSetupInput(root), /path/)
})
