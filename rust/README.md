# FUZE Rust 原型（不接入稳定版）

目标是最终全 Rust 应用，本目录包含 M0 与独立 M1a 原型。先读 [迁移清单](../docs/development/rust/PLAN.md)、[接口契约](../docs/development/rust/CONTRACT.md)、[验收标准](../docs/development/rust/ACCEPTANCE.md)、[M0 验证结果](../docs/development/rust/RESULTS.md) 与 [M1a 后端和原始画面验证](../docs/development/rust/M1A.md)。

- fuze-contract：三槽位、未知轮次、会话/版本/序号接纳规则。
- fuze-ocr：真实 ONNX CPU / DirectML 标题识别、BGRA ROI、CTC 解码、严格名称匹配和一次性 CPU 回退。固定单行区域限制明确，不是完整截图识别替代品。
- fuze-native-overlay：纯 Rust Win32/GDI 绘制。默认无可见窗口；隐藏 smoke 只绘制与采集自己创建的窗口，报告 PrintWindow / WM_PRINTCLIENT 的实际结果。--show 由用户主动运行演示，不抢焦点，无常驻刷新计时器。未自动在游戏上方验证。

Windows 上需要 Rust 1.90.0 MSVC、C++ 工具与 Windows SDK。工具链可使用隔离 RUSTUP_HOME/CARGO_HOME，不改系统 PATH。

    cd rust
    cargo fmt --all -- --check
    cargo test --workspace --locked -j 2
    cargo build --workspace --release --locked -j 2
    cd ..
    node scripts/check-rust-prototypes.mjs
    node --test scripts/test-rust-host.mjs
    node scripts/compare-rust-ocr.mjs

OCR CLI 可选参数为 `cpu`、`dml`、`auto`、`dml-strict`，随后可选测试张量导出目录。dml 允许模型的少量 CPU 辅助算子，dml-strict 只作兼容性诊断。默认比较不强制要求 GPU；显式设置 `FUZE_RUST_GPU_TEST=1` 开启 GPU 对比与独立执行 provider 审计。原始张量/profile 只保留在被忽略的测试输出目录，均不上传。

fixture 脚本从现有模型 YAML 导出临时字符表，并按该 PP-OCRv5 模型补充末尾空格类别，仅使用已经提交的三个 OCR fixture。默认显式使用现有 node_modules/onnxruntime-node 中的 DLL，可用 FUZE_RUST_ORT_DLL 指向受信副本；伴随 DLL 目录只加入测试子进程 PATH。不读取运行程序、真实缓存、日志或游戏。Node 是测试准备工具，Rust 可执行文件本身不需要 Node/Electron。

浮窗接受 `--smoke 输出PNG 快照JSON` 或用户主动执行的 `--show 快照JSON`。验证脚本将部分识别样本的真实输出传给隐藏浮窗；不使用预填名称冒充识别，也不显示编造的胜率。错误阶段代号：0 参数/初始路径，1 运行库/词典输入，2 模型初始化，3 预热，4 fixture 输入，5 推理/解码，6 期望结果不符。崩溃或启动失败可能没有阶段代号。

检查结果在 rust/artifacts，目录不提交。CLI 不打印原始识别文本或本机路径。不要把未验证的性能或原生窗口样式当成游戏实测。ONNX Runtime/模型沿用既有依赖与许可说明；Windows 绑定为 MIT/Apache-2.0。模型和 DLL 不复制进新源码目录。
