# Skia 性能与窗口验证 · 2026-10-16

结论：继续验收 Skia 硬件主界面，Skia 软件表面作为降级候选。旧软件渲染的主要可消除开销与字体文件反复访问有关；不以额外常驻字体内存作为生产默认修复。中文结构与品牌不变；稳定版、生产宿主与依赖均未替换，没有安装包。

## 同字体对照

Slint 1.13.1 的 `i-slint-core/software_renderer/fonts/vectorfont.rs` 在字体建立、文本 shaping、字符查找时使用 `with_face_data`。fontdb 0.23.0 的文件 Source 在该访问期间打开并映射文件，回调结束后释放映射；已有 glyph 缓存并不能消除所有这些访问。此为固定依赖源码检查与受控 A/B 共同支持的瓶颈判断，未宣称采样分析已取得每个函数的耗时百分比。

同一原型、同一字体字节、同一隐藏 Winit 光栅化路径，每组 12 次热绘制：

| 测量 | 系统字体文件 | 进程内保留相同字体 |
|---|---:|---:|
| 热绘制中位数 | 162.76 ms | 7.96 ms |
| P95 | 249.35 ms | 8.90 ms |
| 12 次绘制进程 CPU 时间 | 1906.25 ms | 93.75 ms |
| 192 次字体数据访问 | 56.94 ms | 0.0155 ms |

内存方案实际保留 6 个文件、8 个字体 face、40,558,240 字节（约 38.68 MiB），准备 13.74 ms；单文件上限 32 MiB，总预算 48 MiB。文件不写出，原字节内容不改。两个最终 PNG 经完整字节比较一致，并实际查看中文排版；图片不重复提交。证据：[file](performance/font-file.json)、[resident](performance/font-resident.json)。短样本用于定位，不代表所有页面或长期帧率。

内部 FONT_DB 接口只留在隔离原型中；未来 Slint 更新可能改变该行为。Skia 本身已更快，没必要为了主界面默认额外复制约 39 MiB 字体。

## 实际 Skia 表面与图形设备

自有 HWND 始终隐藏；自定义 Slint WindowAdapter 实际调用 Skia render/present，区别于 Winit take_snapshot。当前上下文返回 NVIDIA GeForce RTX 3060 Ti，OpenGL ES 3.2 NVIDIA 617.14。该证据仅确认此探针实际设备；普通可见 Winit 窗口的设备仍由人工入口另行采集。

1120×860、100% 的 12 次热绘制中位数：

| 路径 | 呈现前 | 呈现及完成等待 | 合计 |
|---|---:|---:|---:|
| Skia OpenGL | 2.61 ms | 14.02 ms | 16.58 ms |
| Skia software | 3.68 ms | 0.05 ms | 3.74 ms |

各列独立取中位数，不能要求中位数之和精确等于合计。OpenGL `glFinish` 仅用于这个有界基准，不进入交互/生产绘制循环。约 16.6 ms 周期与呈现/驱动同步等待相符，但未通过驱动跟踪证实具体同步机制；呈现前计时也不等于 GPU 执行完毕。不能凭此说 GPU 比 CPU 慢，或声称游戏帧率改善。

还检查 150% 逻辑缩放和 920×760 尺寸，各轮结束后 20 次无变化 draw 请求检查均为零重绘。这是有限同步探针，不代替长时间可见事件循环空闲采样。主动 suspend 后重建上下文成功；不是模拟真实设备丢失。

原始计数与边界：[OpenGL](performance/skia-surface-opengl.json)、[Skia software](performance/skia-surface-software.json)。无输入模拟、游戏访问或桌面采集，前台窗口比较未变化。

## Winit 与人工边界

真实隐藏 Winit 的 920×760 / 1120×860 尺寸请求与客户区尺寸核对通过。Skia snapshot 热绘制中位数 2.96 ms，Skia software 为 3.52 ms；这些是 CPU 光栅化快照，不当作 GPU 帧。证据：[Skia](performance/native-skia.json)、[Skia software](performance/native-skia-software.json)。

新增 `--manual-profile` 只由用户手动启动可见原型。记录绘制、大小、DPI、焦点、IME 事件计数，以及图形设备与会话 CPU 时间；不记录输入内容、按键、鼠标位置、路径、账号或画面。绘制回调耗时最多收集前 512 个样本，不含完整呈现时间，不能推算整段会话的 GPU 帧率；全程事件计数不截断。非正常退出可能没有报告。

[人工清单](../MANUAL_ACCEPTANCE.md) 的 IME、跨屏、最小化恢复、RDP、显示器移除和可见空闲行为目前均未验证。没有自动 GPU 回退、生产托盘或业务接线。人工验收和 CI 完成之前不放行全面迁移，也不打包。

## 验证范围

- 本机 release + gpu 编译成功；上述固定场景分别以独立进程完成，最终退出码均为 0。
- 已检查最终字体 A/B 图片一致；没有启动可见手动入口。
- Windows CI 增加隐藏 Skia software 表面检查；托管 runner 不强求物理 NVIDIA/OpenGL。CI 通过也不能替代本机可见窗口验收。
- 前一原型 PR #49 的 Windows 检查已通过；本次后续 PR 单独检查，无发布和安装步骤。
