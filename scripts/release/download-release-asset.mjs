import {createHash} from 'node:crypto';

// The public filename is reused when the update channel switches. Read the exact
// asset whose ID, size and digest were returned together by the release API.
export async function downloadReleaseAsset(api, asset, timeoutMs) {
  const response = await fetch(`${api}/releases/assets/${asset.id}`, {
    headers: {Authorization: `Bearer ${process.env.GH_TOKEN}`, Accept: 'application/octet-stream'},
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) throw Error(`Download ${asset.name} HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const digest = 'sha256:' + createHash('sha256').update(bytes).digest('hex');
  if (bytes.length !== asset.size || digest !== asset.digest) throw Error('Digest mismatch ' + asset.name);
  return bytes;
}
