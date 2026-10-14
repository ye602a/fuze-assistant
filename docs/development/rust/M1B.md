# M1b：DirectML 固定输入与原生采集原型

本阶段基于 PR #19，分支为 feat/rust-native-capture。只修改隔离 Rust 原型、验证工具和原型 CI；不接入、安装、重启或替换 Electron 稳定版，不修改推荐排序。

## 适配器与慢点结论

- DirectML 显式使用 device_id=0，按 DXGI EnumAdapters1 顺序报告名称、索引、软件标志和显存容量，不输出 LUID/硬件序列号。微软说明 DirectML device_id 对应 DXGI EnumAdapters 顺序；不能用 CIM 或任务管理器的显示编号代替。
- 2026-10-15 本机 DXGI 索引 0 为 NVIDIA GeForce RTX 3060 Ti，8024 MiB，非软件适配器。索引 1 也显示同名设备，不能据此推断有两张物理显卡；索引 2 为软件渲染器。OCR 明确传入 0，独立 ORT provider 审计确认 DML 执行。
- PP-OCRv5 输入为 [DynamicDimension.0, 3, 48, DynamicDimension.1]。原路径为各标题产生不同宽度，只预热 256；一次不同尺寸的预热不能消除实际形状的准备成本。
- 同一 Rust 二进制 A/B 确认主要性能问题在动态尺寸路径，不是换语言本身的收益，也不是虚拟显卡误选。调度、图优化和传输各自占比没有硬件 trace，不能给出更细的精确归因。
- Node ORT 此版本设置 freeDimensionOverrides 后仍返回动态输入元数据，因此撤掉该无效对照。最终 A/B 使用 Rust，并在创建会话后验证 [1,3,48,512] 已真正生效。

官方依据：[DirectML 配置与性能](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)、[WGC CreateForWindow](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow)。

## 实现

| 项目 | 行为 |
| --- | --- |
| GPU 输入 | 保持字形比例、原始高度；NCHW 各行右侧补零到 512，固定 batch=1、width=512，校验实际 shape |
| 超宽输入 | 不截断或强行压缩；auto 经已有单次回退释放 GPU 后转 CPU；显式 dml 不隐藏失败 |
| CPU 输入 | 保留动态宽度，避免无谓处理填充区域 |
| 预热 | GPU 在实际 512 尺寸执行两次，CPU 一次；成功后重复调用直接返回。初始化/预热独立计时 |
| 资源 | 一个 OCR 会话；不建立各宽度会话池，不逐帧创建会话，不持续 profiling |
| GPU 回读 | 三块 ROI 复制到一个 staging atlas，只 Map 一次，按 RowPitch 拷贝，复用 staging 和 CPU buffer，无图片编码 |
| WGC 原型 | 只接受当前进程的隐藏 HWND；禁用指针采集；2 个 frame buffer，通知容量 1，最多等待通知 750 ms，有限取最新帧 |
| 清理 | Drop 注销通知并关闭 session/pool；帧用完 Close；Map 成功后显式返回路径均 Unmap |
| 守卫 | 限制尺寸/ROI/缓冲区预算；尺寸变化返回 resize_requires_recreate，不沿用旧坐标 |

WGC 目前是一次性探针，尚不是常驻采集服务。设备丢失恢复、分辨率变化重建、可见自有窗口及游戏取帧仍未验证。WGC 探针读取三个小测试区；GPU 回读单独使用标题大小的 ROI 校验全部像素。没有声称打通 WGC → OCR → 窗口实战链路。GPU Map 等驱动调用本身不具备 750 ms 通知等待的时间上限；宿主测试进程仍有总超时保护。

## 本机有限测量

同模型、同 DLL、同 GPU、同一 Rust 可执行程序；三个已有 fixture（完整三卡、部分识别、无识别），每槽位单独预热后测三次，共 27 次。输入内容相同，填充路径增加零列。没有控制其他应用负载，CPU 时间粒度约 15.625 ms；只用于方向判断。

| 路径 | 单标题推理中位数 | 27 次进程 CPU 时间 | 第一组三卡全槽处理 |
| --- | ---: | ---: | ---: |
| CPU 动态宽度 | 14.16 ms | 531.25 ms | 38.83–40.04 ms |
| DML 动态宽度 | 47.95 ms | 1093.75 ms | 112.20–136.83 ms |
| DML 补齐 512、模型仍动态 | 7.48 ms | 125.00 ms | 27.48–28.16 ms |
| DML 补齐且模型固定 512 | 3.92 ms | 31.25 ms | 16.36–17.80 ms |

四种路径期望槽位全部一致。固定 GPU 初始化 942.13 ms、预热 11.11 ms；另一次初始化约 1.48 秒。冷启动须提前完成，不能将热运行数字当作首次打开耗时。三卡处理含 ROI/归一化/推理/解码匹配，不含截图等待、文件解码、IPC、窗口展示。

固定尺寸独立 provider 审计两次预热看到 2 个 DML 事件、0 个 CPU 事件，DML 事件共 3415 微秒。M1a 动态路径一次预热为 299 个 DML、2 个 CPU 事件；事件数量不是 GPU 占用率或直接可比的总耗时。

生成的 1920×1080 GPU 纹理：整帧为 8,294,400 字节，三个标题区域 104,064 字节（约 1.25%）；每帧一个 Map。首次回读约 0.88 ms，后续约 0.10–0.15 ms，所有像素校验通过。没有截屏或读取游戏。

WGC 在本机 create_hidden_window_item 阶段不可用，没有获得 WGC 帧。隐藏窗口的 WM_PRINTCLIENT 回读通过，但不冒充 WGC 成功。测试窗口始终隐藏，foreground_unchanged=true，500 ms 空闲 CPU 计时为 0（受计时粒度限制）。

## 验证与复现

使用隔离 Rust/MSVC/SDK，Windows 绑定锁定 windows 0.61.3；不改系统 PATH/注册表，Cargo 依赖留在已授权隔离目录。

~~~powershell
cd rust
cargo fmt --all -- --check
cargo test --workspace --locked -j 2
cargo build --workspace --release --locked -j 2
cd ..
node scripts/check-rust-prototypes.mjs
node scripts/check-rust-capture.mjs
node --test scripts/test-rust-host.mjs
# 有硬件 GPU 时手动执行，不在无 GPU CI 强制性能断言
node scripts/compare-rust-shapes.mjs
~~~

FUZE_RUST_BIN_DIR 可指向隔离 exe 目录。FUZE_RUST_DML_SHAPE 仅供离线对照，支持 dynamic/padded512/fixed512，默认 fixed512。CPU 仍为 CLI 默认；auto 是故障回退，尚非速度自适应选择。

7 项 Rust 边界/契约测试、2 项宿主测试、本机 release 编译、CPU/DML 离线样本与生成纹理回读通过。Windows CI 复核构建与无 GPU 回退；不跑整套 Electron 测试或生产安装包构建。

测试输出只在被忽略的 rust/artifacts；CI 允许上传脱敏报告、自绘 PNG、三个独立测试 exe 和校验清单。原始 fixture 图片、张量、profile、日志、工具链、凭据、个人缓存不随本次 PR 上传。

## 下一阶段放行条件

1. 可见自有测试窗口的 WGC 取帧、尺寸变化重建、设备丢失与连续释放验证；本机目前只完成隐藏窗口失败边界。
2. 将 GPU ROI atlas 接至 OCR，取消中间整帧复制，测量捕获到绘制的端到端延迟。
3. 更多标题长度、缺失卡位、分辨率和语言样本验证 padding512 精度；三个 fixture 不替代完整验收。
4. 用户主动实战验证 GPU 竞争与功耗后，再决定是否默认 GPU。稳定版和整体胜率推荐继续保留。
