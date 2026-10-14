# M1c：原生标题回读接入 OCR

基于 PR #20 的隔离原型。没有替换正式入口，没有读取运行中的游戏、客户端、个人缓存或日志；本机不运行可见窗口测试。

## 已完成

- 抽取共享 titles 几何与借用 Atlas 契约。原始 BGRA、测试张量和 GPU 回读使用同一标题坐标；严格保留左/中/右槽位。
- GPU 三 ROI atlas 直接进入 OCR，只把三个小区域转 RGB；不把小区域还原成整帧，不进行图片编解码。上传 fixture 到 GPU 是离线输入准备，不冒充 Windows 捕获。
- 同一 GPU readback 对象连续处理不同尺寸纹理：按 atlas 尺寸更换 staging，常规帧复用分配。
- 新增 OwnedWindowStream：本进程 HWND 校验、2 帧池、1 个通知位、最新帧优先、时间戳去重、借用当前结果。超时/失败后禁止读取上次 atlas。
- ContentSize 改变时先释放已取帧，再 Recreate frame pool，递增采集 generation、丢弃 staging 和旧时间戳。新布局重新取得有效帧后才供 OCR 使用。
- Map 使用 DO_NOT_WAIT，GPU 忙时短暂休眠并使用 20 ms 软截止时间；避免卡在阻塞 Map 或忙等。调度/驱动调用仍不是硬实时保证，宿主保留总超时。
- WinRT apartment 与流绑定在原线程，COM/帧池先释放，最后 RoUninitialize。
- 可见自有窗口的 WGC 取帧/改变尺寸测试只允许显式 GitHub CI 模式；本机默认隐藏。报告区分可用、无帧与验证成功。

## 本机结果

8 项 Rust 契约/边界测试、release 构建通过。三个已有 fixture 经过 GPU atlas → CPU OCR 和 GPU atlas → DirectML OCR 两条路径，18 次全槽识别均符合预期；部分卡位仍保持 unreadable。两条结果分别传入自有隐藏窗口，绘制与前台不变检查通过。

本机每帧 atlas 为 104,064 字节；有界非阻塞回读约 2.7–4.0 ms。DirectML 三卡 OCR 约 16.1–17.3 ms。这个回读路径比 M1b 的阻塞 Map 微基准更慢，换取可检查的等待边界；后续应评估异步 readback 双缓冲，而不是用忙等压低数字。

这些耗时不包括真实 WGC 帧等待、数据源、IPC、可见展示。标题 fixture 范围有限，几何仅覆盖已有固定单行布局；不是所有分辨率、中文标题或真实游戏验收。

## 验证

已有 Rust 格式/单测/release 构建后运行 node scripts/check-rust-pipeline.mjs。FUZE_RUST_BIN_DIR 可选隔离 exe 目录。脚本仅使用已有 fixture 与模型，不调用 LCU，不枚举游戏。

Windows CI 设置 FUZE_RUST_CI_VISIBLE=1，仅自有窗口允许短暂显示/调整尺寸；没有模拟鼠标键盘。若 CI 没有 WGC 合成能力，报告必须保持 verified=false，不以其他回读方法冒充成功。设备恢复、长时间资源稳定性、真实游戏捕获仍待下一阶段。

只上传脱敏 JSON、自绘窗口图片和隔离 exe；不上传原始 fixture、张量、profile、日志或工具链。稳定版不安装、不重启、不发布。
