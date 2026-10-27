import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import AdmZip from 'adm-zip';
import {verifyBinaries} from '../../scripts/release/verify-binaries.mjs';

const bytes = Buffer.from('synthetic desktop, not executable');
const receipt = {'fuze-ui-lab.exe': createHash('sha256').update(bytes).digest('hex')};
test('release receipt verifies the formal player name without requiring the Cargo name', () => {
  const zip = new AdmZip();
  zip.addFile('FUZE.exe', bytes);
  assert.doesNotThrow(() => verifyBinaries(zip, {desktopExecutable:'FUZE.exe'}, receipt));
  zip.addFile('FUZE.exe', Buffer.from('changed'));
  assert.throws(() => verifyBinaries(zip, {desktopExecutable:'FUZE.exe'}, receipt), /Build binary mismatch/);
});
test('missing formal executable cannot pass using a leftover old executable', () => {
  const zip = new AdmZip();
  zip.addFile('fuze-ui-lab.exe', bytes);
  assert.throws(() => verifyBinaries(zip, {desktopExecutable:'FUZE.exe'}, receipt), /Build binary mismatch/);
  assert.doesNotThrow(() => verifyBinaries(zip, {}, receipt));
});
