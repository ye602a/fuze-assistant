# Fuze 应用更新与增量下载

## 当前渠道

Fuze 使用私有仓库 `ye602a/fuze-assistant` 的 GitHub Releases，与原项目的数据 API 分离。入口为 `src/main/app-update-service.ts`。更新凭据仅在本机使用，通过 Electron safeStorage 加密保存，不随源码或发行文件分发。

自动下载和退出时自动安装均关闭。用户主动检查、下载、重启安装；选人、加载、对局和重连阶段阻止更新下载及安装，进入相关阶段取消已有下载。NSIS 发行文件包括完整 `.exe`、`.exe.blockmap`、`latest.yml` 和 `SHA256SUMS.txt`。

PR 和手动 CI 的 Actions 测试产物不是正式发布。正式版本通过版本脚本创建提交和标签，经过检查、打包、扫描、实际窗口就绪验证后发布。见 [贡献约定](../AGENTS.md) 和 [窗口验收](RELEASE_WINDOW_SMOKE.md)。

## 增量下载现状

NSIS 框架支持通过 blockmap 复用旧安装包数据块，只下载变化部分，然后组合并校验完整安装包。这减少网络下载量，仍需安装并重启，不是对局中热替换代码。

项目锁定 `electron-updater 6.8.9`，本次源码检查不能证明私有频道增量下载正常工作：

- `PrivateGitHubProvider.resolveFiles` 返回带数字资产 ID 的 GitHub API 地址。
- 该 provider 未覆盖 `getBlockMapFiles`；继承实现对文件 URL 拼接 `.blockmap` 并替换版本号。
- 数字资产地址不能如此转换为对应 blockmap 资产地址。库的差分失败路径会回退完整下载。

因此有 blockmap 只是准备条件，当前仍应按可能完整下载理解。本次未进行两套正式安装包的升级实测，也未修改依赖 provider。

## 后续实施与验收

独立 PR 应解析新旧发布中的 blockmap 资产 ID，验证带认证的 Range 下载和重定向边界。旧安装包缓存缺失、blockmap 缺失或损坏时回退完整下载。复用 updater 原有完整性校验，避免叠加重复的全文件哈希遍历。

必须用真实旧版到新版升级验证实际下载字节数、最终校验、对局阶段取消与恢复行为；同时覆盖令牌失效和网络中断，保证不泄漏认证信息。不能用单元测试推算节省比例。

清理时保留历史正式 Releases、标签和 blockmap。重复 Actions 测试产物不承担更新分发，可以清理。

参考：[electron-builder 自动更新文档](https://www.electron.build/v26/docs/features/auto-update/)。以锁定版本和实际升级验证为准。
