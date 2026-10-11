# Fuze 私有频道增量更新

应用更新来自私有仓库 `ye602a/fuze-assistant`，与推荐数据更新分离。检查、下载和安装均由玩家主动发起；退出软件不会自动安装。选人、加载、对局及重连期间禁止下载和安装，阶段切换会取消下载。

## 下载方式

从包含此实现的版本开始，更新优先使用旧安装包缓存与新旧 blockmap，只请求变化的数据块，重建完整 NSIS 安装包后由 updater 执行 SHA-512 校验。它节省的是下载量，安装时仍需退出符择；不会对局中热替换代码。

没有旧安装包缓存时直接完整下载，跳过差分元数据请求。blockmap 缺失、损坏、缓存不匹配或服务器不支持正确 Range 响应时回退整包；完整包仍必须通过校验。取消下载不会触发整包回退，下一次下载需玩家主动点击。

**从 v0.5.0 或更早版本升级到首个增量版本，仍可能完整下载。** 执行下载的是旧版本更新器，新能力不能在安装前生效。增量大小依版本内容、打包结果和缓存情况而变，不承诺固定节省比例。

## 实现边界

- `src/main/update/private-provider.ts` 通过准确的发布资产名称解析新旧 blockmap 的资产 ID，不对数字 API URL 拼后缀。
- `private-update-executor.ts` 使用隔离网络会话，在每次重定向时创建新请求。认证仅发往指定仓库的 GitHub API；CDN 请求移除认证、Cookie 和 staging ID。只接受固定 GitHub API 路径与两个资产域名，禁止降级到 HTTP。
- 同一下载期间复用资产重定向结果，结束或取消后清空。顺序 Range 下载避免多范围兼容问题；响应必须是匹配范围的 206。
- `private-nsis-updater.ts` 保留上游重建和完整性校验，补充无基线快速回退、取消保护和旧 pending blockmap 清理。不新增运行期整文件哈希扫描。
- 凭据通过本机 safeStorage 保存；不进入 renderer 状态、日志、下载地址、构建或发行包。自动下载与自动安装保持关闭。
- 内部扩展基于锁定的 `electron-updater 6.8.9`。升级依赖必须重跑以下回归，不能假定内部接口不变。

## 验证

`node scripts/test-private-update.mjs` 在隐藏、隔离的 Electron 进程内使用虚构 HTTPS 响应，覆盖差分、无缓存、缺失/损坏 blockmap、旧缓存损坏、Range 被忽略、校验失败、非法重定向、取消和同实例重试。所有安装/重启入口在测试中禁用。

`node scripts/test-private-update-live.mjs` 使用有仓库读取权限的临时环境凭据，或本机已有 Git 凭据，下载真实 v0.4.2 基线，以当前实现重建真实 v0.5.0 包；仅下载与校验，不执行安装。测试固定历史版本以免最新发布指针改变测试目标，记录下载字节数和请求数。CI 通过临时 GitHub token 执行，不保存凭据。此下载验证不等同于玩家本机完整安装验收。

PR、正式标签的 Windows CI 均运行上述验证，以及源码/安装包凭据扫描和实际打包窗口检查。正式版本还需完整发布 `.exe`、`.exe.blockmap`、`latest.yml`、`SHA256SUMS.txt` 和发行说明，核对 latest 指针。清理时保留历史正式发布和 blockmap。

参考：[GitHub 资产 API](https://docs.github.com/en/rest/releases/assets)、[electron-builder 更新文档](https://www.electron.build/v26/docs/features/auto-update/)。
