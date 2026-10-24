import {readFileSync} from 'node:fs'
import {test} from 'node:test'
import assert from 'node:assert/strict'

test('new packages and Setup share the 0.7.X version; historical repack stays 0.7.0',()=>{
 const version=readFileSync('rust/app-version.txt','utf8').trim()
 assert.match(version,/^0\.7\.(0|[1-9]\d*)$/)
 const setup=readFileSync('installer/rust-setup.iss','utf8')
 assert.ok(setup.includes(`#define AppRelease "${version}"`))
 assert.ok(setup.includes('AppVersion={#AppRelease}'))
 assert.ok(readFileSync('scripts/package-rust-setup.ps1','utf8').includes('/DAppRelease=0.7.0'))
})
