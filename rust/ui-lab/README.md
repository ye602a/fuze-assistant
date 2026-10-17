# FUZE 原生界面

当前本目录用于 Rust 测试版的 Slint 主界面和 Direct2D / DirectWrite 三卡浮窗，代码已随 [#95](https://github.com/ye602a/fuze-assistant/pull/95) 合入 master。`ui-lab` 是沿用的目录名，不表示当前下载包仍只是界面原型。玩家下载见 [Rust 测试频道](https://github.com/ye602a/fuze-assistant/releases/tag/rust-preview)。

- Slint 主界面保留中文版侧栏、大状态区、四个模块和三卡结构，沿用深蓝、金色与品牌资源。
- 三卡浮窗使用 Direct2D / DirectWrite，固定左右顺序，提供缺失卡位提示、说明、拖动和位置记忆。
- `gpu` 构建使用 Skia，可用 `skia-software` 进行软件渲染对照。工具链与依赖版本以锁文件及 CI 为准。
- `--review*` 和 `--desktop-smoke` 使用隔离数据，不连接玩家游戏；生产启动会按偏好运行真实宿主，不能当成无副作用的开发检查。
- 中文输入法、跨屏、远程桌面和实战窗口行为的验证，与隐藏窗口和 CI 结果分开记录。

[evidence/](evidence/README.md) 保留早期原型、性能调查与阶段检查结果。其中“下一步”“未打包”等是当时状态，不代表当前缺口。当前维护流程见[开发索引](../../docs/development/README.md)，下载包功能以对应发行说明为准。

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
