# FUZE 全 Rust 迁移：范围与实施顺序

状态：原型阶段；稳定版 v0.6.3 与现有 Electron 发布流程继续保留。最终应用业务和界面使用 Rust；ONNX Runtime、Windows API 等原生依赖不要求重新实现为 Rust。

## 功能迁移清单

| 能力 | 当前来源 | Rust 目标 / 保留行为 | 本轮 |
| --- | --- | --- | --- |
| 客户端发现 | services/lcu/*discovery*、token-loader | 进程优先、安装信息回退、手动最终回退；401/404/超时不算连接成功 | 契约，未连接游戏 |
| LCU 与阶段监听 | services/lcu、game-session | 认证仅后台内存、断线重连、世代隔离、只读状态 | 契约 |
| 截图 | capture-stream、screenshot | Windows 原生捕获，低频门禁、ROI、有限队列、黑帧/设备恢复 | 下一阶段；本轮只读测试图片 |
| 中文 OCR | image-analyzer、ocr-worker | 既有 PP-OCRv5 模型、三个位置不重排、预热、串行推理、失败可回退 | CPU 标题识别原型 |
| 匹配/推荐 | image-analyzer、data-loader | 本地字典、版本隔离、明确未知、整体胜率默认 | 小型离线目录与固定卡位输出 |
| 等级门禁 | augment-level-gate、live-level-reader | 等级只指导检测，不证明实际领取；不能饿死低频兜底 | 边界已规定，未迁移 |
| 正式领取轮次 | stage-signal | actualStage=null；碎片、锻造器、刷新不推进轮次 | 契约强制未知 |
| 本地数据与实时更新 | data-loader、recognition-data-service | 缓存优先、三语言隔离、完整后原子切换、来源与版本可见 | 后续 |
| 主界面/设置/状态 | Display、workbench-health | 四色健康状态、数据版本、暂停、模块实际状态 | 后续 |
| 三卡/选人浮窗 | window-manager、renderer overlays | 原位三卡、缺失槽位、非激活置顶、五人/备选区分 | 纯 Rust Win32 三卡原型 |
| 战绩/本地记录/清理 | match-history、storage | 无身份上传、缓存边界、保留/清理规则、会话隔离 | 后续 |
| 托盘/快捷键/语言 | tray、preload、i18n | 用户触发动作、三语言、窗口恢复 | 后续 |
| 出装/符文页可选操作 | item-set-installer、LCU 写接口 | 与只读推荐隔离；保留明确用户开关，不在原型接入 | 后续单独审查 |
| 更新/差分/凭据 | app-update-service、private-update | 私有下载授权、签名验证、差分失败整包、对局不安装 | 后续；不能假设兼容 Electron blockmap |
| 安装/数据迁移/回滚 | app-paths、installer、CI | 新旧独立数据目录，显式迁移、旧版可回退 | 后续 |

## 架构决定

1. 新代码在 rust/ 独立 Cargo workspace，不接入现有 npm 构建、打包或运行入口。无新旧双重采集。
2. fuze-contract 不依赖 GUI/推理库；fuze-ocr 只接受本地输入；fuze-native-overlay 只消费脱敏推荐快照。
3. 首轮 Win32/GDI 原型验证原生窗口行为；不是把 Win32/GDI 直接确定为最终全应用 UI。Iced 等框架仍需后续适配验证。避免在首轮同时引入大型 UI 框架和更换推理后端。
4. OCR 先使用已有识别模型的固定标题 ROI + CTC 解码，不声称复现原版检测网络、模糊匹配与所有画面布局。原型的明确不足写入 ACCEPTANCE.md。
5. GPU、原生游戏捕获、真实游戏浮窗验收不在本轮启动。原型不联网、不开端口、不查进程、不读游戏内存、不模拟输入。
6. 跨进程协议只传命令和小快照；大图像后续采用有界二进制传输或共享缓冲区，不用 JSON/base64 往返。

## 分支与 PR 顺序

- M0（本 PR）：清单、契约、验收规则；Rust 编译锁文件；两个独立原型和离线验证结果。
- M1：同素材识别等价与性能对照；捕获/GPU 后端原型；原生 UI 框架试验。过门槛后才选最终实现。
- M2：LCU、状态机、调度、缓存与数据源。旧程序仍为正式渠道。
- M3：原生主界面、选人窗口、三卡浮窗、设置、国际化。移除过渡桥接后才称全 Rust 应用。
- M4：安装迁移、私有更新、差分、签名和回滚；Windows 隔离打包与实战验收后独立版本发布。

M0 不合并为稳定版发布、不修改稳定安装包。后续阶段每个 PR 必须附证据与未通过项。日常只跑相关测试；阶段结束统一完整检查，避免重复 OCR 基准。本轮原型即使离线通过，也不能自动开启阶段胜率排序。

## 工具与依赖

Rust MSVC 工具链和 Cargo 缓存允许放在 FUZE/.tools/rust；不改系统 PATH。仓库提交 Cargo.lock，不提交工具链、编译产物、模型副本、截图日志或私人目录。ORT DLL 从显式受信路径加载；原型不下载 DLL。

参考：[ONNX Runtime](https://onnxruntime.ai/docs/)、[Rust ort](https://docs.rs/ort/2.0.0-rc.10/ort/)、[Windows Rust 绑定](https://github.com/microsoft/windows-rs)、[Windows 捕获 API](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow)。
