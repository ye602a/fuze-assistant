> 最新开发进度与集中验收清单见 [选人操作与按需图片](evidence/CHAMPION_ACTIONS.md)，此前记录见 [战绩、观察记录与显式迁移](evidence/RECORD_ACTIONS.md)、[启动与维护页面](evidence/STARTUP_MAINTENANCE.md)、[HOST_INTERACTIONS.md](evidence/HOST_INTERACTIONS.md) 和 [LIVE_OVERLAYS.md](evidence/LIVE_OVERLAYS.md)。本批只做轻量检查，完整生产迁移仍未通过，不发布安装包。

# FUZE UI 技术验证

当前进行 [v0.6.3 中文界面恢复](evidence/RESTORATION_PROGRESS.md)。首页结构、资源与三卡尺寸先恢复；设置、战绩和选人是分阶段迁入，未接回的操作明确禁用。此前性能图片是历史原型，不能当作当前界面完成证明。开发侧对照完成前不要求玩家验收，也不打包。

这是独立开发入口，不是新版本或安装包。现已复用 Rust 宿主的 LCU、截图、OCR、偏好与资料服务，并增加显式生产启动及维护入口。`--review*`、`--desktop-smoke` 使用隔离合成数据，不连接游戏；`--desktop 配置文件` 会按照已保存偏好运行真实宿主，仅供空闲时主动验证，开发检查不会启动该模式。界面结构以中文版 FUZE 为基准，英雄候选网格继续属于选人窗口，不混入主界面。

首轮 [本机证据](evidence/README.md) 保留作基线。后续 [Skia 性能与窗口验证](evidence/PERFORMANCE_FOLLOWUP.md) 已定位软件字体访问开销，并验证隐藏 Skia 实际图形表面与上下文重建。整体仍为“原型部分验收通过，全面迁移未放行”；下一步按 [人工验收清单](MANUAL_ACCEPTANCE.md) 验证中文输入和真实跨屏。

- Slint 主窗口：原侧栏、大状态区、四模块、三卡；保留深蓝、金色和品牌资源。首页、偏好与维护、战绩和英雄资料页面已逐步接线；未完成操作仍明确禁用，剩余范围见最新进度文档。
- Direct2D / DirectWrite 浮窗：三卡顺序固定、左侧图片、右侧选取率和胜率、粗进度条、缺失卡位占位；不依赖 Slint 创建浮窗。品牌图片仅为清楚标记的渲染占位，不冒充符文图标。
- `gpu` 构建的交互入口默认选择 Skia；可显式选择 `skia-software` 对照。未启用该特性时仍使用 Slint software。固定 Slint 1.13.1 与现有 Rust 1.90 工具链配合，不以版本号代替兼容性验收。

## 开发运行

从本目录使用已有 Windows Rust/MSVC/SDK 环境：

```powershell
cargo build --release --locked --features gpu -j 2
cargo fmt -- --check
./target/release/fuze-ui-lab.exe --review artifacts
./target/release/fuze-ui-lab.exe --review-native software artifacts
./target/release/fuze-ui-lab.exe --review-native skia artifacts
./target/release/fuze-ui-lab.exe --review-native skia-software artifacts
./target/release/fuze-ui-lab.exe --review-skia-surface opengl artifacts
./target/release/fuze-ui-lab.exe --review-skia-surface software artifacts
./target/release/fuze-ui-lab.exe --profile-fonts software file artifacts/font-file
./target/release/fuze-ui-lab.exe --profile-fonts software resident artifacts/font-resident
```

`--review` 只生成原型自己的离屏画面，创建并销毁隐藏自有窗口。不会截取桌面/游戏或模拟输入。`--review-native` 初始化真实 Winit 窗口，始终隐藏、不调用组件 show/run；其 snapshot 是光栅化，**不是 GPU 帧耗时**。每种后端单独进程运行。

`--review-skia-surface` 在独立的隐藏 HWND 上实际 render/present，记录图形驱动与提交前/呈现等待耗时；不等价于可见 Winit 窗口测试。`--profile-fonts` 是同字体文件/内存副本对照实验，副本最多 48 MiB，只驻留本进程；不会导出字体或改变生产默认。内部 Slint API 只用于本验证工程。

以下命令**仅供玩家空闲时手动启动**，会显示原型，但不会连接游戏：

```powershell
./target/release/fuze-ui-lab.exe --interactive software
./target/release/fuze-ui-lab.exe --interactive skia
./target/release/fuze-ui-lab.exe --manual-profile skia artifacts/manual
./target/release/fuze-ui-lab.exe --overlay
```

三卡窗口不激活、不抢键盘焦点；右上“关闭”退出。输入不写盘。Slint 原型“关于原型”保留 Slint 归属显示；后续发布前需按选定许可证再审核依赖与归属，不因为全 Rust 就忽略第三方许可。

## 验收边界

| 项目 | 自动检查 | 必须人工/真实环境验收 |
|---|---|---|
| 中文 | 简繁、中英混排、换行、字符串模型回读 | 微软拼音/第三方 IME 候选框、组合输入、删除和选中、Tab 焦点、远程输入 |
| DPI | 100/125/150/175/200% 离屏图，920×760 小窗口 | 真实混合 DPI 跨屏、显示器拔插、RDP 重连、Windows 文本放大 |
| 性能 | 各自固定样例的热绘制分位数；空闲重绘计数；隐藏浮窗短时间 CPU 采样 | 同机器、同场景、同采样时长的可见软件/Skia及旧版对比；GPU 实际适配器与占用 |
| 窗口 | 隐藏、不激活、工具窗/置顶样式；前台句柄仅比较，不输出 | 可见主窗口最小化/恢复/托盘；浮窗与游戏无边框模式共存；排他全屏覆盖能力 |
| 恢复 | 主动释放并重建目标资源 | 真正设备丢失、驱动变化、休眠、远程图形后端降级 |

主界面与浮窗的内容和面积不同，计时不可直接拿来判定框架快慢。Direct2D 隐藏硬件目标可能被遮蔽而跳过实际显示，不能用它的提交耗时声称游戏帧率改善。短时间 CPU 计数为 0 也只代表落在进程时间采样精度以下。

## 全面迁移门槛

1. 原型在本机中文输入和真实 DPI/窗口行为通过，且不抢游戏焦点。
2. 可见窗口同口径测量无持续空闲重绘；主界面隐藏时停止无必要绘制，浮窗仅数据/暴露/DPI变化重绘。设定性能预算后再比较，不拿隐藏窗口数据充当性能达标。
3. 根据验收选择 Slint 后端与降级策略；优先验证 Skia 硬件与 Skia 软件备选。当前手动选择后端 **不是已完成自动 GPU 回退**。
4. 下一 PR 仅替换 UI 适配层，沿用现有 Rust 状态/命令契约；先主页面再选人与战绩，功能对照 v0.6.3，候选区按“五人＋网格”例外保留。
5. 实际领取轮次仍未知；推荐继续整体胜率。三卡缺失提示不意味着已证明遮挡原因。
6. 阶段验收完成才制作测试安装包。原型 PR 不触发打包，不修改稳定版本、生产依赖锁或运行配置。
