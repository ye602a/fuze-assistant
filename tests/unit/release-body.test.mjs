import test from 'node:test';
import assert from 'node:assert/strict';
import {releaseBody} from '../../scripts/release/release-body.mjs';
const options = {version:'0.7.11', source:'a'.repeat(40), notes:'- 修复浮窗显示。', buildRun:123, buildSetup:false};

test('update channel explains its purpose and preserves download addresses', () => {
  const body = releaseBody({...options, updateChannel:true});
  assert.ok(body.startsWith('## FUZE 在线更新文件'));
  assert.ok(body.includes('/releases/latest'));
  assert.ok(body.includes('/rust-preview/fuze-rust-preview-aaaaaaaaaaaa.zip'));
  assert.ok(body.includes(options.notes));
  assert.ok(!body.includes('Setup.exe'));
});

test('stable publication has its own copy and numbered assets, with optional Setup', () => {
  for (const buildSetup of [false,true]) {
    const body = releaseBody({...options, buildSetup});
    assert.ok(body.startsWith('## FUZE v0.7.11'));
    assert.ok(body.includes('/rust-v0.7.11/FUZE-0.7.11-windows-x64.zip'));
    assert.equal(body.includes('/rust-v0.7.11/FUZE-Setup-0.7.11-windows-x64.exe'), buildSetup);
    assert.ok(!body.includes('Pre-release'));
    assert.ok(!body.includes('rust-preview'));
    assert.ok(!body.includes('测试版'));
    assert.ok(body.includes(options.notes));
  }
});
