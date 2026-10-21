# 开发资料

[玩家首页](../../README.md) · [玩家指南](../PLAYER_GUIDE.md)

**接续开发先读：[当前状态与 Codex 交接](../CODEX_HANDOFF.md)。** 其中按已实现、已发布、实际未验收、未闭环目标和已取消项整理了本轮迁移。不要把历史阶段报告重复转成待办。

## 当前代码与发布边界

`master` 已通过 [集成 PR #95](https://github.com/ye602a/fuze-assistant/pull/95) 纳入 Rust 开发历史。当前程序为 Rust v0.7.0；v0.6.3 Electron 源码已归档，历史发布继续保留。合并源码、通过 CI 与发布安装包是三个不同步骤，不按每个 PR 自动打包。

| 目录 | 用途 |
| --- | --- |
| `rust/crates/` | LCU、截图、OCR、数据、调度与宿主服务 |
| `rust/ui-lab/` | 当前 Slint 主界面及 Direct2D / DirectWrite 三卡浮窗；目录名沿用原型阶段 |
| `legacy/electron/` | 归档的 Electron / Vue v0.6.3 源码、专属测试与构建配置 |
| `scripts/`、`tests/` | 数据准备、隔离验证、发布与维护脚本 |
| `docs/development/rust/`、`rust/ui-lab/evidence/` | 迁移设计、阶段证据与历史检查记录 |

Rust 构建需 Rust 1.90.0 MSVC、C++ 编译工具和 Windows SDK。以 [Rust 工作区](../../rust/README.md)、[原生界面入口](../../rust/ui-lab/README.md)及当前 Actions 工作流中的命令为准。根目录 Node 工具使用 `npm ci --ignore-scripts` 安装、`npm test` 检查；复现 Electron 请在独立目录检出旧版标签。

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
