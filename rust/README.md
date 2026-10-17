# FUZE Rust 原生测试版

当前开发代码已通过 [集成 PR #95](https://github.com/ye602a/fuze-assistant/pull/95) 合入 master。玩家入口为 [Rust 测试频道](https://github.com/ye602a/fuze-assistant/releases/tag/rust-preview)；原生主界面位于 [ui-lab](ui-lab/README.md)。v0.6.3 Electron 稳定包继续保留，源码合并不自动更新已发布包。

下文及所链接的阶段文档包含原型时期记录。当前结构、验证与发布边界以[开发索引](../docs/development/README.md)和当前工作流为准；历史“下一步”不表示功能仍未实现。

最新界面进度：[中文工作台复原](docs/WORKBENCH_RESTORATION.md)、[中文选人与局内浮窗复原](docs/POPUP_RESTORATION.md)。识别链路见 [M17 分类、刷新保留和加载预热](../docs/development/rust/M17_SELECTION_PREWARM.md)。WGC 可见自有窗口测试仅在 CI 显式运行，不能当作游戏验收；退出问题及防护见 [WGC shutdown](../docs/development/rust/WGC_SHUTDOWN.md)。

目标是最终全 Rust 应用。本目录已包含独立宿主、原生界面、更新和安装维护代码；早期原型文档是历史记录，当前缺口以替换进度表为准。设计边界见 [迁移清单](../docs/development/rust/PLAN.md)、[接口契约](../docs/development/rust/CONTRACT.md) 和 [验收标准](../docs/development/rust/ACCEPTANCE.md)。

- fuze-contract：三槽位、未知轮次、会话/版本/序号接纳规则。
- fuze-runtime：会话失效、等级节能、单在途任务与最新帧决策。
- fuze-worker-host：持久子进程、有限二进制像素管道、一次 CPU 恢复和 Windows Job；生产预览宿主串联阶段监听、游戏窗口捕获、OCR、推荐、原生主界面/托盘/设置/战绩、数据更新及安装维护。显式离线 smoke 入口与生产入口隔离。
- fuze-data：明确根目录的只读准备、语言隔离、不可变快照激活。事务式下载、更新和有界清理由宿主模块承担。
- fuze-lcu：只读 loopback 接口、进程与命令行凭据发现、WebSocket、重连、阶段/英雄变化、账号隔离的历史解析。凭据只在进程内使用，不输出身份或认证 URL。
- fuze-capture：DXGI 适配器映射、原生 WGC 生命周期、三标题 ROI 回读与异常恢复；CI 自有窗口证据不等于本机腾讯对局实测。
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
