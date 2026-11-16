// Latest may itself be an online-only release. Search numbered releases instead.
export async function retainedDownloads(request, api) {
  let zip, setup;
  for (let page = 1; ; page++) {
    const releases = await request(`${api}/releases?per_page=100&page=${page}`);
    for (const release of releases) {
      if (release.draft || release.prerelease || !/^rust-v0\.7\.\d+$/.test(release.tag_name)) continue;
      for (const asset of release.assets) {
        if (asset.state !== 'uploaded' || !asset.size) continue;
        const version = release.tag_name.slice(6);
        const item = {version, url: asset.browser_download_url};
        if (asset.name === `FUZE-${version}-windows-x64.zip` && (!zip || Number(version.split('.')[2]) > Number(zip.version.split('.')[2]))) zip = item;
        if (asset.name === `FUZE-Setup-${version}-windows-x64.exe` && (!setup || Number(version.split('.')[2]) > Number(setup.version.split('.')[2]))) setup = item;
      }
    }
    if (releases.length < 100) break;
  }
  if (!zip) throw Error('No retained full download found');
  return {zip, setup};
}
