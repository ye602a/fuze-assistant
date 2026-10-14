# FUZE Rust 原型（不接入稳定版）

目标是最终全 Rust 应用，本目录只是 M0。先读 [迁移清单](../docs/development/rust/PLAN.md)、[接口契约](../docs/development/rust/CONTRACT.md) 与 [验收标准](../docs/development/rust/ACCEPTANCE.md)。

- fuze-contract：三槽位、未知轮次、会话/版本/序号接纳规则。
- fuze-ocr：真实 ONNX CPU 标题识别、CTC 解码、严格名称匹配。固定单行区域限制明确，不是完整截图识别替代品。
- fuze-native-overlay：纯 Rust Win32/GDI 绘制。默认无可见窗口；隐藏 smoke 仅绘制自己的位图，--show 由用户主动运行演示，不抢焦点，无常驻刷新计时器。未自动在游戏上方验证。

Windows 上需要 Rust 1.90.0 MSVC、C++ 工具与 Windows SDK。工具链可使用隔离 RUSTUP_HOME/CARGO_HOME，不改系统 PATH。

    cd rust
    cargo fmt --all -- --check
    cargo test --workspace --locked -j 2
    cargo build --workspace --release --locked -j 2
    cd ..
    node scripts/check-rust-prototypes.mjs

fixture 脚本从现有模型 YAML 导出临时字符表，仅使用已经提交的三个 OCR fixture。默认显式使用现有 node_modules/onnxruntime-node 中的 DLL，可用 FUZE_RUST_ORT_DLL 指向受信副本；不读取运行程序、真实缓存、日志或游戏。Node 是测试准备工具，Rust 可执行文件本身不需要 Node/Electron。

检查结果在 rust/artifacts，目录不提交。CLI 不打印原始识别文本或本机路径。不要把未验证的性能或原生窗口样式当成游戏实测。ONNX Runtime/模型沿用既有依赖与许可说明；Windows 绑定为 MIT/Apache-2.0。模型和 DLL 不复制进新源码目录。
