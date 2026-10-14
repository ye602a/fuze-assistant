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

验证结果在本 PR 的 Windows 检查与后续追加测量记录中列出；未通过的项目不得写成成功。
