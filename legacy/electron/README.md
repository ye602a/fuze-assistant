# Electron v0.6.3 archive

这里保存被 Rust 新版替代的 Electron/Vue 源码、专属测试、开发脚本和构建配置。
旧 `package.json` 和锁文件原样保留，版本仍为 0.6.3。

该目录用于查阅和排查历史行为，不参与当前 Rust 程序的构建或运行。
如需复现旧版构建，请在独立工作目录检出原始 `v0.6.3` 标签；归档目录不是独立发行工程，
其历史脚本使用当时的仓库布局。旧工作流保存在 `.github/workflows/` 子目录中，GitHub 不会执行它。

新版仍需要的图标、OCR 模型与测试样本保留在仓库根目录的 `public/`、`resources/` 和
`tests/fixtures/`。旧 JS 数据下载器及其接口说明也归档在本目录的 `scripts/`、`docs/` 下；
下载器复用归档内的 `src/shared/client-data-security.ts`。当前构建通过 Rust 下载器准备数据。
历史 Releases 和更新地址保留。
