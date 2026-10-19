# PR #109 合并后验证与未使用代码清理

更新：2026-10-19。基线 `3d2f697e066d3015e04e793e6da8f24815815f70`。
PR #109 的 Windows、Rust 核心、Rust UI、运行库契约四项检查全部通过后合并。
本次未打包、发布、安装运行库、启动或切换 FUZE。
公开包仍为 `e941c7e124c7`，不包含 #109 的安装前检测入口。

## 真实构建回执与安装前检测

在固定到上述合并提交的干净 detached worktree 中，使用独立空 target 目录运行
`scripts/build-rust-with-evidence.ps1`，核心 release 编译 2m54s、Slint/Skia release
编译 9m13s，均成功。使用现有 MSVC 与隔离 SDK 链接库；Skia 使用其构建脚本取得的
预编译依赖，未安装系统 SDK、运行库或 FUZE。

实际回执为 schema 1、completed/sourceClean=true、revision=`3d2f697e066d`、
frontend=slint；编译器 `19.44.35229.0`、链接器 `14.44.35229.0`。
策略 SHA-256 为 `f1782ad3e0873e04db7f91495c510d81a25e83b297d70dd3ca1e29897e0da9e8`。

| 自有文件 | 本次编译 SHA-256 |
|---|---|
| fuze-rust-host.exe | `018d224c325ae2bd2d073364d669527563b983db60a889cef3fb4db00410797d` |
| fuze-ocr-worker.exe | `fbb1656f3c1b643e8fa567b7b5eb6e6614a486e994fcde50d0a101a765594785` |
| fuze-update.exe | `2c1b3a40d7030e36f429238e0d80ab993b5d358d27294f83da07ea0212b44717` |
| fuze-ui-lab.exe | `397e7989f3a838f9f60c6eb5bc0ff0250358b1d73c1b66e85bdce55003558706` |

仅将 4 个自有 EXE、4 个固定第三方 DLL、build.json 和 helper 复制成静态检查 fixture，
没有制作 ZIP/FUZ，也没有把它作为可用测试包交付。生产 `makeRuntimeContract` 接受该实际
回执及 8 个 x64 文件，得到 5 个 CRT 导入：msvcp140、msvcp140_1、msvcp140_atomic_wait、
vcruntime140、vcruntime140_1。错误提交、未完成、脏源码标记、错误策略、错误前端、
超出支持基线的链接器和被改动的二进制共 7 类输入均拒绝。

生产 `Install-local.ps1 -CheckOnly` 对该 fixture 返回 `runtime-required` / exit 2：
本机 5 个 x64 CRT 文件均为 `14.50.35719.0`，低于项目支持基线 `14.51.36247.0`。
这证明检测会阻止不满足项目支持策略的安装，不证明旧运行库必然无法加载程序。
微软固定安装器的散列、有效签名与 FileVersion 校验通过；其已提取的 5 个 x64 DLL 均为
`14.51.36247.0`，传给同一判断函数得到 ready。只读取文件元数据，没有加载 DLL 或安装。

旧 build.json 在真实 CheckOnly 入口保持 unknown-build / exit 2；修改副本 EXE 后
返回 exit 1，恢复后哈希不变。另以必定失败的 Cargo 替身调用真实 build wrapper，
确认旧 completed=true 回执先失效为 completed=false，编译失败不会保留成功回执。
完整编译使用真实 Cargo；失败注入只验证错误路径，二者证据分开记录。

本机详细复验脚本、编译回执与有限状态报告保存在
`C:/Users/HANO/Documents/FUZE/reviews/post109-validation-20261019/`；不含玩家日志。
这些结果补齐构建与静态入口验证，不等于签名安装、运行时加载或实机安装通过。

## 清理依据与结果

检查了基线 1,306 个跟踪文件的入口与引用，覆盖 `src/` 的 228 个 TS / JS / Vue
文件、166 个 Rust 源文件、8 个核心 crate 加独立 UI 的依赖，以及脚本、测试、构建配置。
从 Electron main / OCR worker / preload / renderer 四个构建入口追踪静态与字面量动态
import、别名、Vue 外置样式及 CSS import；另核对路由、HTML 入口、工具入口和历史用途。
没有把“没有 import”直接等同于无用：声明文件、Cargo 自动 examples/tests、CLI 和兼容
入口分别处理。静态扫描不证明所有真实窗口和玩家场景已验收。

删除 74 个文件，主要是没有生产入口的孤立实现与它们独占的依赖：

| 删除项 | 依据 / 仍保留的生产路径 |
|---|---|
| React context / reducer / actions、旧 localStorage config、未使用的符文/来源常量、Jest setup | Vue 生产入口没有调用；React / Immer / Jest-dom 已不在项目依赖中。未清除用户 localStorage 或配置 |
| AutoScreenshotConfig、ChampionMonitor、FeedbackWidget、ItemSetInstaller、OverlayPreferences、PostGameShareModal、StatCard、WinrateOverlay | 四条生产路由均未挂载；Display、UpdatePanel、StoragePanel、战绩、AugmentWinrateOverlay 和三卡实际路径保留 |
| 未挂载的 use-app-update / use-post-game-share、旧 winrate 服务、旧截图压缩与样式、组件 utils | 仅被上述孤立组件引用或没有调用。主进程更新、反馈、赛后数据和隐私门禁仍在生产链中 |
| components/ui 的 alert / badge / button / card / input / select / skeleton / table / tabs 及独占 class 工具 | 唯一外部引用来自已未挂载的 ChampionMonitor；其余没有生产消费者 |
| renderer/services/lcu/types.ts | 无调用；生产 LCU 类型来自 main/services/lcu/types.ts 与 shared 契约 |
| security/update-trust.ts | 只有孤立单测调用，当前更新生产路径使用 update/request-policy.ts、private-provider / executor；没有删除实际的下载域名、散列、重定向或安装门禁 |

移除 6 个直接开发依赖：`@vue/tsconfig`、`@vueuse/core`、`reka-ui`、
`class-variance-authority`、`clsx`、`tailwind-merge`。锁文件删除 22 个包记录；
npm 10 同时补齐 6 个既有 Tailwind WASM 可选依赖的嵌套记录。剩余既有记录版本与元数据
保持原样，7 个运行依赖未变。`vue-tsc` 虽不被源码 import，仍是 type-check 命令，保留。
安装时出现的传递依赖弃用提示不等于可以删除其消费者；本次没有强制升级 Electron、
Slint、图标库或跨版本合并 Windows crate。

移除 4 个仅覆盖删除文件的测试文件；赛后测试保留主进程不恢复/显示主窗口的断言。
英雄窗口生命周期测试改为读取实际主进程偏好实现。主进程偏好、监控状态、IPC 权限、
更新请求策略与窗口生命周期测试保留。清理后生产入口可达的 146 个模块仍在。

## 有明确用途而保留的内容

- v0.6.3 Electron 生产源码与稳定发布，Rust Slint/Skia、Direct2D/DirectWrite、DXGI/GDI
  默认采集链不变；WGC 诊断代码没有接回默认链。
- `lcu-utils.ts`、LCU barrel、renderer file 安全空实现及 logger：显式兼容入口。
- `analyze-q4.ts`、`lcu-debug.ts`、OCR 标题匹配参考与离线测试：诊断/迁移对照，不在本次运行。
- Rust examples、软件渲染、ui-audit/font probe、Win32 检查/回滚/诊断二进制：有明确工具或
  CI 用途，不按 lab/probe/legacy 名称删除。166 个 Rust 文件未发现可据本次证据删除的孤岛。
- 模型、公开图标、字体、许可证及数据文件由包白名单、动态资源路径或生成代码使用；
  不能用 JS import 扫描判定资源冗余。用户数据、配置和历史报告未做破坏性清理。

## 清理验证

- 在全新隔离目录用最终锁文件执行 `npx -p npm@10 npm ci --ignore-scripts`，成功。
- 同一干净依赖目录：85 个单测文件 / 468 项测试、lint、type-check、生产 build 均通过。
- 运行库契约 / 发布就绪 / 包白名单 / CI 约束：24 项 Node 测试通过，其中执行 Windows
  PowerShell 的 30 项安全状态断言及 `-CheckOnly` 入口。未启动 FUZE 或安装器。
- 本机两份受保护未提交笔记未修改、未纳入提交。

## 需要用户安排的实机部分

仍需独立安排真实 UAC、无/旧/x86 运行库电脑、重启后复检、原始 ZIP 安装入口、旧受管包
在线升级、健康失败回滚和再启动；以及同版本实战、IME、混合 DPI、真实窗口与副屏边界。
签名还需要正式发布者身份、公开证书指纹与安全渠道授权，见
[VC 运行库引导与签名清单](VC_RUNTIME_BOOTSTRAP.md)。不要把这些静态检查当作正式版通过。
