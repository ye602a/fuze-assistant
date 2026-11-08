const repository = 'https://github.com/ye602a/fuze-assistant';

export function releaseBody({version, source, notes, buildRun, buildSetup, updateChannel = false}) {
  const revision = source.slice(0, 12);
  const base = `${repository}/releases/download/${updateChannel ? 'rust-preview' : `rust-v${version}`}/`;
  const zip = updateChannel ? `fuze-rust-preview-${revision}.zip` : `FUZE-${version}-windows-x64.zip`;
  const setup = updateChannel ? `FUZE-Setup-${version}-preview-${revision}-windows-x64.exe` : `FUZE-Setup-${version}-windows-x64.exe`;
  const introduction = updateChannel
    ? `## FUZE 在线更新文件

此页面供 FUZE 客户端检查和下载更新。普通下载请前往[最新正式版](${repository}/releases/latest)。

这里的 Pre-release 标记用于保留在线更新入口，不代表另一个测试版本。当前提供 **${version}** 的在线更新文件。`
    : `## FUZE v${version}`;
  return `${introduction}

${buildSetup ? `- [下载安装包（Setup.exe）](${base}${setup})` : '可在 FUZE 设置中检查更新并安装。'}
- [下载便携 ZIP](${base}${zip})

所需 VC++ DLL 已随程序提供。便携包请完整解压后运行 \`FUZE.exe\`。

### 本次更新

${notes}

版本 **${version}** · 构建：\`${revision}\` · [对应源码](${repository}/commit/${source})

ZIP 和在线更新包来自同一提交。

[构建记录](${base}build-evidence-${revision}.json) · [构建工作流](${repository}/actions/runs/${buildRun})
`;
}
