# 开发资料

[玩家首页](../../README.md) · [玩家指南](../PLAYER_GUIDE.md)

**接续开发先读：[当前状态与 Codex 交接](../CODEX_HANDOFF.md)。** 其中按已实现、已发布、实际未验收、未闭环目标和已取消项整理了本轮迁移。不要把历史阶段报告重复转成待办。

## 当前代码与发布边界

当前应用版本以 `rust/app-version.txt` 为准，已发布版本以 Releases 为准。Electron 源码已从当前分支移除，历史源码可通过 `v0.6.3` 标签查阅，历史发布继续保留。应用版本不使用根目录 `package.json` 的工具包版本；普通 PR 不自动发布安装包。

| 目录 | 用途 |
| --- | --- |
| `rust/crates/` | LCU、截图、OCR、数据、调度与宿主服务 |
| `rust/ui-lab/` | 当前 Slint 主界面及 Direct2D / DirectWrite 三卡浮窗；目录名沿用原型阶段 |
| `scripts/`、`tests/` | 数据准备、隔离验证、发布与维护脚本 |
| `docs/development/rust/`、`rust/ui-lab/evidence/` | 迁移设计、阶段证据与历史检查记录 |

Rust 构建需 Rust 1.90.0 MSVC、C++ 编译工具和 Windows SDK。以 [Rust 工作区](../../rust/README.md)、[原生界面入口](../../rust/ui-lab/README.md)及当前 Actions 工作流中的命令为准。根目录 Node 工具使用 `npm ci --ignore-scripts` 安装、`npm test` 检查；复现 Electron 请在独立目录检出旧版标签。

数据准备使用 `node scripts/prepare-rust-data.mjs`，`npm run prepare:client-data` 指向同一入口。
先构建 `fuze-rust-host.exe`；脚本从 `rust/target/release` 或 `FUZE_RUST_BIN_DIR` 查找该工具，
以 `--prepare-data` 写入仓库的 `resources/client-data`，不启动桌面或连接游戏。
旧 Electron 专属 JS 下载器已移除。

公开数据读取、程序更新和对局上传的地址及范围见[数据接口说明](../client-api-strategy.md)。
对局贡献模拟仍在独立的 [PR #127](https://github.com/ye602a/fuze-assistant/pull/127)，不包含在这次 0.7.1 数据更新改动中。

## 验证与发布

功能走分支和 PR。按改动选择必要验证，大阶段统一验收；文档修改不需要重复进行游戏链路测试或打包。

- `rust-prototypes.yml`：Rust 核心契约、OCR、采集、回退与生命周期。
- `ui-framework-lab.yml`：控件、正式宿主回调、窗口恢复、截图至生产弹窗。
- `release-windows.yml`：新版工具与源码检查，保留原分支保护检查名称，不再打包 Electron。
- 专项诊断工作流保留为手动入口。普通 PR 不发布安装包。

测试截图必须使用已提交夹具或自建窗口。单元测试、隐藏窗口、CI 和实战是不同证据层级。实际领取轮次未确认，自动推荐保持整体统计。

## 资料索引

- [迁移清单](rust/PLAN.md)、[接口契约](rust/CONTRACT.md)、[验收标准](rust/ACCEPTANCE.md)
- [阶段交接记录](../RUST_TEST_STAGE_HANDOFF.md)、[测试玩家说明](../RUST_PLAYER_TEST_GUIDE.txt)
- [分阶段统计](../STAGE_STATISTICS.md)、[领取信号验证](../STAGE_SIGNAL_VALIDATION.md)
- [贡献约定](../../AGENTS.md)、[来源与致谢](../../THANKS.md)
- [旧版架构](ARCHITECTURE.md)、[旧版更新设计](../ELECTRON_APP_UPDATE_STRATEGY.md)、[旧版安装包验收](../RELEASE_WINDOW_SMOKE.md)

历史原型、计划和验收文档保留供追溯，不应把其中“下一步”“未打包”或“私有仓库”视为当前发布状态。最新下载状态以 Releases 为准，当前行为以源码和对应提交的检查结果为准。

不得提交真实日志、账号信息、个人截图、令牌或本机配置。公开更新无需令牌；自动创建诊断 Issue 仍需用户明确授权，不得内置共享凭据。

## 发布与编译缓存

新编号版本使用 Actions 的 `Publish numbered Rust release`，仅从 master 手动运行。
更新 `rust/app-version.txt`、Setup 版本和 `docs/RELEASE_NOTES.md` 后经 PR 合并；已发布的版本在编译前终止，不重做同版本 Setup。
发布复用 master 范围的 Rust 缓存，不再每次建立临时打包分支。发布脚本位于 `scripts/release/`，在线更新仍使用现有公开清单和包格式。
只改 Slint 界面时由 UI 流水线验证，不再触发不含 UI 的核心 workspace 全量编译；服务源码改动仍触发两条相关流水线。
本地继续复用显式指定的共享 target 目录，不为清理空间删除正在使用的编译缓存。
