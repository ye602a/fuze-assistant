# 开发资料

[玩家首页](../../README.md) · [玩家指南](../PLAYER_GUIDE.md)

## 本地开发

使用锁文件安装依赖：`npm ci`。开发入口为 `npm run dev`，构建为 `npm run build`，打包为 `npm run pack`，客户端数据准备为 `npm run prepare:client-data`。

提交前运行 `npm run test:unit`、`npm run lint`、`npm run type-check`、`npm run build` 和 `npm run security:check`。识别与数据改动另运行相关 Electron 回归。发布前必须通过 Windows CI 的实际打包窗口检查；单元测试不能代替真实客户端验收。

## 维护索引

- [架构](ARCHITECTURE.md)、[术语与背景](CONTEXT.md)、[界面设计](DESIGN.md)
- [贡献约定](../../AGENTS.md)
- [性能诊断](../PERFORMANCE_DIAGNOSTICS.md)、[存储管理](../STORAGE_MANAGEMENT.md)
- [分阶段统计](../STAGE_STATISTICS.md)、[领取信号验证](../STAGE_SIGNAL_VALIDATION.md)
- [应用更新与增量下载](../ELECTRON_APP_UPDATE_STRATEGY.md)
- [安装包窗口验收](../RELEASE_WINDOW_SMOKE.md)
- [来源与致谢](../../THANKS.md)

历史检查文档是开发证据，不代表正式安装包包含主分支全部功能。功能走分支和 PR，检查通过再合并；正式版本单独交付。不得提交真实日志、账号信息、个人截图、更新令牌或本机配置。
