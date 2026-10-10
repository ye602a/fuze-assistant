# 符择 Fuze

本地优先的 Windows 海克斯大乱斗助手。源码与发布版本位于私有仓库 `ye602a/fuze-assistant`。

- 从正在运行的 League 客户端自动发现 LCU，界面展示实际连接、游戏阶段、截图、OCR 与数据状态；游戏目录仅作高级回退。
- 持续低帧率截图、原始像素复用、重复帧过滤；OCR 在独立进程运行，WebGPU 预热失败时切换单线程 CPU。
- 使用官方本地只读接口读取等级。普通阶段省电等待，临近强化等级、待领取和重选时恢复检测；接口失效时兼容检测，低频兜底不遗漏特殊触发。
- 独立识别字典与胜率统计分开缓存、更新。胜率不混合不同补丁；历史、未验证和缺失数据明确标注。
- 顶部三卡保持游戏左右顺序；侧边详情、战绩与偏好分别呈现。
- 个人分支关闭遥测及后台战绩上传，不向游戏进程注入或读取游戏内存。

## 安装与更新

从本仓库 Releases 下载 `Fuze-Setup-<version>.exe`。首次安装需要手动运行安装包；早期 ARAMGG 或 Fuze 本地包不会自动切换到这个频道。

设置页的“软件更新”支持私有 GitHub Releases：检查版本 → 手动下载 → SHA-512 校验 → 确认安装。选人、载入和对局阶段暂停下载、禁止安装，退出应用不会自动安装。当前没有 Windows 发布者签名，完整性校验不等于发布者签名；不启用静默自动更新。

私有仓库需要每台电脑独立授权：在 GitHub 创建 Fine-grained personal access token，限定 `fuze-assistant`，仅授予 Contents: Read-only 权限，然后在设置中加密保存。Windows `safeStorage` 将其绑定本机用户；令牌不能写入源码、工作流文件或安装包。不要将开发者写权限令牌分发给测试者。环境变量 `FUZE_GITHUB_TOKEN` 可用于临时自动化测试，优先于已保存授权；取消授权时也需移除此环境变量。

没有授权的用户只能在浏览器登录有仓库访问权限的 GitHub 账号后下载。便携包不原地安装，需下载新目录，并保留旧包作为回退。

## 本地开发与验证

Node 22.18+，npm 10；按锁文件安装：`npm ci --ignore-scripts`。Electron 安装脚本被禁用时，首次开发需执行 `node node_modules/electron/install.js` 下载运行时。

```text
npm run dev
npm run test:unit
npm run lint
npm run type-check
npm run test:augment-ocr
npm run pack
npm run test:packaged-windows
```

打包会准备三种语言的公开数据缓存，失败时停止发布。真实打包窗口检查使用隔离数据，不操作游戏、鼠标或键盘。设置 `ARAMGG_RELEASE_SMOKE_HIDDEN=1` 可隐藏测试窗口；设置 `FUZE_PIPELINE_FIXTURES` 为 `tests/fixtures/augment-ocr` 的绝对路径，可验证 GPU、CPU 回退和截图链路。

`build/`、`dist/`、`dist-electron/`、用户缓存、配置和日志不提交。发布使用 `npm run release:minor` 或 `npm run release:patch` 创建版本提交与附注标签，再推送本仓库；Windows 工作流验证通过才发布安装包及 `latest.yml`。工作流不启用原作者的上传服务。

## 数据与来源

识别字典：ARAMMayhem 公共 CC BY 4.0 数据，完整归属信息随缓存保存；腾讯公开静态数据仅提供版本参考。胜率主源沿用 ARAMGG 客户端公开数据通道，不能把静态版本号当成本机版本实测，也不能把识别字典新版本当成新胜率。

基于 `valkia/aramgg_client` 开发，保留上游提交历史与第三方声明，详见 [THANKS.md](THANKS.md)。本仓库不对未明确授权的上游素材另行授予许可。

离线测试不能代替腾讯国服真实对局验收。升级后需实测三卡出现、重选、领取后关闭、游戏结束以及远程桌面下的截图兼容性。
