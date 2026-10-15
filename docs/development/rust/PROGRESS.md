# Rust 替换进度（2026-10-15）

**正式运行入口尚未切换，当前安装的稳定版仍使用 Electron/Vue。** 下表按已实现和验证边界列出进度，不用代码行数或编译通过率冒充迁移百分比。PR 按依赖顺序 #18 → #19 → #20 → #21 → #22 → #23 → #24 → #25 → #26 审查。

| 模块 | Rust 进度 | 仍缺少 |
| --- | --- | --- |
| 迁移清单、消息契约 | 已有版本/世代/三槽位/未知轮次契约 | 生产宿主接线、完整用户数据协议 |
| OCR 推理核心 | CPU、DirectML、真实模型、固定输入、预热与一次 CPU 回退已有离线验证 | 完整检测/语言/模糊匹配/刷新保留逻辑与广泛画面验证 |
| 原生捕获 | 三 ROI GPU 回读已接入 OCR；CI 自有窗口取帧/重建；针对已出现的退出访问冲突补系统 DLL 生命周期防护 | 退出防护复核、设备恢复、真实游戏/多屏/DPI/长期验收 |
| 三卡原生浮窗 | Rust 自绘、三槽位、缺失卡位；M5a 接持续 OCR 更新、独立消息线程、跨版本/旧结果拒收与结束清空 | 正式推荐统计与交互、真实置顶和最终 UI 框架 |
| 会话/等级/调度 | M2a 决策核心；M2b 持久 worker/像素协议；M2c 一次 CPU 恢复、Windows Job 归属、离线 Session 接线 | 真实 WGC/LCU/浮窗事件循环接线、长期运行及 UI 异步桥接 |
| 实际领取轮次 | 强制未知 | 可靠正式领取信号；继续整体胜率 |
| LCU/客户端发现 | M4a 凭据边界、固定只读 WinHTTP；M5a 原生进程路径/lockfile 发现、原进程句柄绑定、HTTP 轮询恢复与退避，离线测试/编译通过 | 真实国服验证、lockfile 缺失替代路径、WebSocket 重连、硬取消和语言/选人数据 |
| 缓存与数据源 | M3a 只读本地优先、三语言隔离、内存快照；M5a 将同一词典经启动管道传入持久 OCR，浮窗携带数据版本 | 远程准备/落盘事务、多渠道合并、统计排名 |
| 主界面/选人/设置/托盘 | 未迁移 | 完整原生功能和设计验收 |
| 战绩/记录/清理 | 未迁移 | 数据生命周期、个人信息边界、旧数据迁移 |
| 更新/差分/安装/回滚 | 未迁移 | Rust 安装更新格式、签名、凭据、稳定渠道与回滚 |

所有上述阶段均不发布正式安装包。原型 exe 为隔离测试工具，不是可替代稳定版的完整应用。

具体证据与限制见 [M1b](M1B.md)、[M1c](M1C.md)、[M2a](M2A.md)、[M2b](M2B.md)、[M2c](M2C.md)、[M3a](M3A.md)、[M4a](M4A.md)、[WGC 退出处理](WGC_SHUTDOWN.md)。各 PR 的 CI 结论以对应提交的 GitHub 检查为准。

下一阶段先完成真实捕获宿主与 UI 异步桥接、设备恢复，以及只读 LCU 的进程发现/重连；再接本地数据和远程事务、多渠道统计，随后主界面/选人/设置/托盘。最后做安装更新/差分/回滚和完整兼容性验收。自动阶段胜率继续等待可靠正式领取信号。


## M5a update

See [M5a](M5A.md): persistent hidden overlay delivery, startup-only catalog transfer from the prepared local data snapshot, and opt-in native process-path/lockfile discovery with bounded HTTP retry scheduling. Offline integration is implemented; live WGC-to-UI orchestration, production statistics, WebSocket reconnection and hard IO cancellation remain open. Stable application entry is unchanged.
