import {createHash} from 'node:crypto';

// Build receipts use Cargo output names; player packages may rename the desktop.
export function verifyBinaries(zip, build, binaries) {
  for (const [file, digest] of Object.entries(binaries)) {
    const entry = file === 'fuze-ui-lab.exe' && build.desktopExecutable === 'FUZE.exe' ? 'FUZE.exe' : file;
    const bytes = zip.readFile(entry);
    if (!bytes || createHash('sha256').update(bytes).digest('hex') !== digest) {
      throw Error(`Build binary mismatch: ${entry}`);
    }
  }
}
