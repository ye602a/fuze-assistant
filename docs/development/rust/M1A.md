# M1a：原始画面与推理后端

本阶段叠加在 M0 原型分支上，不接入或替换 Electron 稳定版。它验证迁移边界，不代表完整 Rust 应用或实战性能验收。

## 实现与验收

| 项目 | 行为 | 验收依据 |
| --- | --- | --- |
| 原始画面 | 借用 top-down BGRA8 帧，校验宽高、stride、缓冲区长度；只转换三个标题 ROI | padding/越界单测；既有三个 OCR fixture 保持左右槽位 |
| 推理后端 | cpu / dml / auto；顺序执行、单线程、关闭忙等；dml 允许 CPU 辅助算子并显式标记 | 同模型结果一致；独立 provider audit 确认 DML 实际执行；dml-strict 禁止 CPU 图回退作为诊断对照 |
| 故障回退 | auto 遇到 GPU 初始化/预热/推理故障，释放会话后最多一次 CPU 回退 | 状态机单测；宿主超时测试；已发生内部回退时禁止再次重启 |
| 预热 | 初始会话与故障后新 CPU 会话先预热；失败预热本身重试不重复预热 | 代码路径检查；初始化与预热耗时单独报告 |
| 原生采集契约 | 只接收本程序创建的隐藏 HWND，PrintWindow 或显式 WM_PRINTCLIENT 返回 BGRA；不做图片编解码 | 验证内部背景像素、stride 与尺寸，报告实际方法；隐藏合成器表面为空时不能宣称 PrintWindow 成功；不是游戏截图验收 |
| 对比测量 | Rust 输出固定 ROI 张量，Node 使用完全相同张量与 ONNX DLL/模型；每种形状预热一次后测三次 | 分离推理 wall time、进程 CPU time 与全帧处理耗时 |

## 验证命令

Windows SDK 可用的独立编译环境：

```powershell
cd rust
cargo fmt --all -- --check
cargo test --workspace --locked -j 2
cargo build --workspace --release --locked -j 2
cd ..
node scripts/check-rust-prototypes.mjs
node --test scripts/test-rust-host.mjs
node scripts/compare-rust-ocr.mjs
```

在具有实际 GPU 的机器上，显式设置 `FUZE_RUST_GPU_TEST=1` 再运行最后一个命令，执行 DirectML 与 Node DirectML 对比、独立 Rust provider audit 及 dml-strict 诊断。CI 可能没有硬件 GPU，auto 成功不等于 GPU 实测成功。DLL 来自已有 onnxruntime-node 依赖；无需复制模型或修改系统 PATH。相同 DLL 的对比前提是未用 FUZE_RUST_ORT_DLL 覆盖为其他版本。

DirectML 模式允许 CPU 承担模型的少量辅助算子，与整个 GPU 会话失败后重建为 CPU 会话是两回事。报告明确输出 `gpu_graph_cpu_assistance_allowed`；独立审计只汇总 DML/CPU 执行事件计数与耗时，不把“provider 注册成功”当成 GPU 执行证据。审计通过显式 `FUZE_RUST_AUDIT_DIRECTORY` 开启，另建会话且不计入基准测试，不在日常识别中持续 profiling。实际选择哪个物理适配器仍须后续核实，不以 DML 事件冒充显卡利用率采样。

## 测量与隐私边界

- 推理计时不包含 PNG 解码、磁盘 I/O、CTC 解码和名称匹配；不是两个完整应用的性能对比。Windows 进程 CPU 计时有粒度限制，小于一个计时粒度时可能显示零，不能解释成零 CPU 消耗。
- fixture 的 RGB→BGRA 转换只在加载样本时做一次，不在逐帧计时中；生产原生采集应直接提供 BGRA。
- 张量只写到被忽略的 `rust/artifacts` 测试目录，不随 CI artifacts 或 PR 上传。已有 fixture 不是匿名素材，不扩散其图片或原始识别文本。
- 原始 provider profile 同样留在被忽略的测试目录，不上传；只在 GPU 验证显式启用。
- 上传允许列表只有脱敏 JSON、原型自己绘制的窗口 PNG、两个隔离测试 exe 及其 SHA256SUMS。exe 不是正式安装包，不含模型、运行库、凭据或用户缓存。
- GPU 出错日志只保留固定阶段/原因；原生运行库错误原文不上传。超时宿主只终止自己启动的测试子进程。

## 尚未实现 / 不得据此放行

- Windows Graphics Capture、真实游戏窗口采集、设备丢失恢复、DPI/多屏/遮挡与多分辨率测试仍待后续阶段。
- 固定单行标题 ROI、少量样本词典不等价于稳定版完整 OCR 检测、语言路由、缓存、刷新保留逻辑。
- 同一图像的相同 ROI 张量用于隔离语言绑定差异；迁移收益还要测整个采集→ROI→推理→消息→绘制链路。
- 原生界面仍为 Win32/GDI 实验，最终 UI 框架尚未决定。
- 未验证真实领取信号；`actual_stage` 仍未知，不开启分阶段自动排序。

## 2026-10-15 验证结果（香港时间）

原型代码 `b3ba9df`：[Windows run 38603775470](https://github.com/ye602a/fuze-assistant/actions/runs/38603775470) 全部通过。5 项 Rust 单测、2 项宿主定向测试、locked 构建、三组既有 OCR fixture、原生隐藏窗口、同输入 CPU 对比与 auto 回退均通过。隐私扫描 689 个文件，零发现。后续报告字段命名调整不改变 Rust 二进制。

本机缺少 Windows SDK，未增装 SDK。下载了上述成功 CI 的两个独立 exe，校验 workflow head SHA、artifact digest 和 exe SHA256 后，仅在本机执行离线 OCR；没有运行可见浮窗或采集本机屏幕。未安装、重启或替换稳定版。

本机对比采用同一已安装 ONNX Runtime DLL、同模型、同输入张量，每个槽位单独预热后测 3 次，共 27 次推理。数据为一次有限微基准，未控制其他应用负载；不能视为游戏中的 CPU 占用率或完整应用性能。

| 本机路径 | 单标题推理中位数 | 27 次推理的进程 CPU 时间合计 | 识别结果 |
| --- | --- | --- | --- |
| Rust CPU | 14.36 ms | 500 ms | 全部符合预期 |
| Node CPU | 15.03 ms | 702 ms | 全部符合预期 |
| Rust DirectML（含少量 CPU 辅助） | 42.13 ms | 1203.13 ms | 全部符合预期 |
| Node DirectML（同配置） | 42.06 ms | 1200 ms | 全部符合预期 |

Rust / Node CPU 中位数接近，不能凭这组小样本宣称换语言本身带来显著收益。当前 DirectML 路径反而更慢且进程 CPU 时间更高，**不满足设为默认性能优化的门槛**。保留原型 CPU 默认；auto 目前只是“GPU 优先、故障时 CPU 回退”，不代表按性能自动选优，也未接入稳定版。

- Rust CPU 初始化 103.09 ms、预热 20.95 ms；完整三卡样本的三次全槽处理约 40.19–43.32 ms（含 ROI/推理/匹配，不含截图、文件解码与窗口显示）。
- 独立 Rust DirectML 初始化 629.06 ms、预热 599.17 ms；auto 首次试跑初始化 723.27 ms、预热 1197.84 ms，第一张三卡样本首轮约 624.35 ms。GPU 冷启动与新形状准备不能隐去。
- Rust 独立 provider 审计看到 299 个 DML 事件与 2 个 CPU 事件，CPU 辅助事件合计 18 微秒。该审计包含预热，不能与上述热运行逐次计时混算，也不能把事件数当作时间占比。
- dml-strict 在初始化阶段失败；允许辅助算子后的 dml 与 auto 在本机均正确运行，auto 未触发整体回退。CI 的 auto 则在 GPU 初始化失败后仅回退一次 CPU，结果仍正确。
- 本机存在 RTX 3060 Ti 与虚拟显示适配器；当前使用默认 DML adapter。未将其 DXGI 索引与物理设备核对，也未采样物理 GPU 利用率。`directMlExecutionVerified` 只证明 DML 执行事件，`physicalAdapterVerified=false`，不冒充已确认指定显卡承担推理。
- CI 隐藏窗口采集使用明确报告的 WM_PRINTCLIENT 回退，760×170、stride 3040、约 1.85 ms、原始 BGRA 校验通过、编码次数 0。PrintWindow 返回成功但画面验证未过，`print_window_verified=false`；不作为 PrintWindow 游戏兼容证据。
- CI 自有窗口仍隐藏、无激活、前台焦点未改变；工作集约 8.32 MiB。500 ms 空闲 CPU 时间观测为 0，仅是有限采样。

## 本机工具链补充验证

2026-10-15 后续获得用户授权，将微软官方 [Microsoft.Windows.SDK.CPP.x64 10.0.26100.9169](https://www.nuget.org/packages/Microsoft.Windows.SDK.CPP.x64/10.0.26100.9169) 的 x64 UM/UCRT 链接库解压到 FUZE 工作目录内的隔离工具目录。归档 SHA-512 与 NuGet 官方 catalog 一致；未运行系统安装器，未改用户/系统 PATH 或 SDK 注册项。未安装完整 SDK headers/tools。

源码 `72ebcb3` 已在本机以 Rust 1.90.0、既有 MSVC C++ 工具、隔离 SDK 完成 `cargo test --workspace --locked --offline -j 2`（5 项通过）及 Release 构建（OCR 与原生浮窗）。输出使用独立 target-local 目录，没有覆盖 CI 下载产物或稳定版。

本机新编译的 OCR 可正常加载已有 ONNX 运行库和模型，3 组既有素材共 9 次结果全部正确，初始化约 97.62 ms、预热约 21.76 ms。仅为编译/链接/运行功能验证，未重复 GPU 性能测试，也未运行可见窗口或验证游戏。此前“本机缺 SDK、只能使用 CI 二进制”的限制已对当前 Rust 原型解除。

## 下一阶段优先级

1. 先核实 DXGI 适配器与硬件执行，比较动态形状、算子分区和输入传输开销，再决定是否保留 DirectML 或评估其他 Windows 后端。没有证据前不把问题归因于玩家的显卡或驱动。
2. 对标题输入形状分桶与提前预热做受控实验，验收识别准确率及首次/稳定帧耗时；减少模型编译和重复传输，不能只看热运行速度。
3. 实现 Windows Graphics Capture 自有测试窗口原型，再做更多 OCR 布局/遮挡/刷新样本与完整链路对照。继续保留稳定版；实战与物理 GPU 验收仍未完成。
