# GitHub Actions 编译缓存

## 2026-11-15：结论与依据

近期 [PR #243 的 UI 检查](https://github.com/ye602a/fuze-assistant/actions/runs/43705964281) 用时 20分11秒，其中四次 Cargo 编译合计约 17分33秒，Rust 缓存未命中。[0.7.14 在线发布](https://github.com/ye602a/fuze-assistant/actions/runs/43706213043) 已命中旧发布缓存，仍下载约 1.1 GB 并编译 6分47秒。缓存命中本身不能证明省下了多少编译。

此前 core/UI 只在 PR 或手动运行中构建，缓存键还包含每次提交的 SHA。GitHub 将 PR 缓存限制在 `refs/pull/N/merge`，其他 PR 和 master 不能读取；默认分支的缓存可以供 PR 读取。新 PR 因此经常冷启动，单改缓存键无法解决这个范围限制。[GitHub 官方规则](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#restrictions-for-accessing-a-cache)

## 当前方案

- `.github/actions/rust-build-cache/action.yml` 统一恢复规则，使用已有 `actions/cache/restore`，不增加第三方缓存服务或付费容量。
- core、UI、publish 分为三条独立缓存。core release 使用 thin LTO 和单 codegen unit；UI release 使用另一套 profile，审计和生产 features 也不同。发布还使用经过 VC runtime policy 审核的 MSVC 和专属 Rust metadata flags。这些参数保持原样，不能为了复用缓存改变生产二进制的编译策略。
- 键不包含源码提交 SHA。编译身份包含 lane、实际 Rust 1.90 编译器、runner 系统/架构及镜像版本、构建配置；发布另包含 runtime policy。依赖部分涵盖对应工作区的 manifest 和 lockfile，同一编译身份内允许回退到上一份依赖缓存。禁止恢复旧的宽泛 `ui-lab-win-1.90-` 缓存。
- 缓存 Cargo registry/git 的下载资料，以及 release 的 `.fingerprint`、`build`、`deps`。顶层程序、构建回执、截图、打包目录、debug 和 incremental 不进入新缓存；内部编译所需文件不做自行裁剪。
- PR 仅恢复，不再每提交保存一份大缓存。无论是否命中，原有编译、测试和界面验收继续执行，由 Cargo 判断哪些依赖需要重编译。

Cargo 根据 profile、features、flags 等条件区分构建产物；共用一个 target 目录不代表所有依赖只编一次。[Cargo 构建缓存](https://doc.rust-lang.org/stable/cargo/reference/build-cache.html)

## master 基础缓存的建立与恢复

`rust-cache-warm.yml` 在 master 的相关源码/构建配置合并后运行，也保留手动入口。core/UI 各自先使用 `lookup-only` 查询精确键：

1. 已存在：不下载大缓存，不编译。
2. 缺失：恢复相同编译身份下的兼容缓存，编译与 PR 相同的 core/UI/audit/production/OCR targets；测试只使用 `--no-run` 编译。
3. 编译成功：保存 master 缓存，供后续 PR 读取。

预热不执行 FUZE、界面测试、游戏操作、打包或发布，也不安装 npm 依赖。仅可信 master 入口写入基础缓存，不使用 `pull_request_target` 或 PR 产物写入 master。普通源码合并通常只做轻量查询；镜像升级、依赖变化或缓存被逐出时才补建。不同合并的过期预热会取消。

使用整张 runner 镜像版本是保守选择：预装 MSVC/SDK 变化不会误共享编译缓存，但镜像更新会产生一次新的冷构建。正常活跃读取会刷新缓存访问时间，无需新增每日保活任务；GitHub 的容量与闲置逐出规则仍然适用。[缓存逐出规则](https://docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching#usage-limits-and-eviction-policy)

## 发布缓存

发布仍仅允许 master 手动运行。先选择符合政策的 MSVC，再恢复独立 publish 缓存；始终运行 `build-rust-with-evidence.ps1`，保留干净源码、编译身份、回执和二进制哈希校验。编译成功后立即保存缓存，后续数据准备或发布失败不再丢掉已完成的编译。缓存保存失败不阻挡发布校验。

本轮不改应用版本、不触发应用发布。新缓存命名空间第一次运行仍需冷构建；只有 master 预热完成后的后续 PR 才能验证跨 PR 的实际收益。以日志中的命中键、恢复/保存耗时和各 Cargo 编译耗时比较，不预先承诺固定提速比例。

## 本轮不采用的方案

不强行合并 core/UI/publish profiles，不跳过生产或审计入口，不复用旧构建回执。暂不引入 sccache 或 Swatinem/rust-cache：后者的清理按工作区执行，FUZE UI 目标目录还容纳来自 core workspace 的 OCR worker，直接双工作区裁剪同一目录容易丢掉另一方依赖；本轮用稳定缓存键与主分支供给即可解决已确认的问题。
