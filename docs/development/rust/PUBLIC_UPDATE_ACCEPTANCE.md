# 公开 Rust 更新验收

日期：2026-10-19。目标公开包：`1c2281940730`；旧包：`56ca08c45a63`。
本轮只编译开发验收工具，未打包、发布、启动 FUZE、登记安装或切换玩家版本。

## 身份与调用链

公开 [Rust 频道](https://github.com/ye602a/fuze-assistant/releases/tag/rust-preview)
的 `rust-update.json` 指向序号 `1792343832`、68,163,364 字节的
`Fuze-Rust-1c2281940730.fuz`。包身份由清单和包内 `build.json` 确定，
不使用滚动标签所指的历史提交替代。

新包 SHA-256：
`3fdb44935bace3d8f0a916556f145705feab2abaae29d2e5502eea1e32655693`。
旧包 SHA-256：
`6cf6c957d19ad86d3cc8a3526d6cda5f096d52c7edb8fcb204bebf4b94c5b010`。
旧基线来自已保存的发布资产，使用前与当前 GitHub 资产摘要重新核对。

正式更新入口为 `fuze-update --update` → `Session::public/check` →
`fetch_observed` → Range / 全量与 `update_resume::Cache` →
`managed_install::unpack/check_bundle/activate` → 下次 `launch_with`。
本轮验证复用其中的生产传输、解包和指针函数。
从旧包到目标包，以及目标包到 PR #105 合并后的主分支，这些生产更新实现没有变化。
本轮新增的是验收入口与报告字段，没有另写更新器或改变玩家更新门禁。

## 不启动应用的验收入口

`fuze-update-verify.exe` 是开发工具，不进入玩家包。以下命令只使用显式输入：

```text
fuze-update-verify.exe --transport-public NEW_ROOT OLD.fuz OLD.fuz.blockmap
fuze-update-verify.exe --resume-seed-public NEW_RESUME_ROOT
fuze-update-verify.exe --resume-public SAME_RESUME_ROOT
fuze-update-verify.exe --stage-only NEW_MANIFEST NEW.fuz OLD_MANIFEST OLD.fuz NEW_STAGE_ROOT
```

- `--transport-public`：不读取 stdin / 本机凭据；真实差分下载后，以无效旧 blockmap
  触发一次真实全量回退，两份输出均通过清单 SHA-512；保存受限清单供后续复核。
- `--resume-seed-public` / `--resume-public`：前一进程真实下载 1 MiB 后主动中断，
  后一进程读回同一清单的缓存，下载后缀并验证整包。此处主动中断是验收注入，
  不是声称已遇到真实断网。
- `--stage-only`：必须使用不同版本和全新目录。校验两包大小、SHA-512，
  使用生产解包器验证路径、配置与构建身份，再检查待确认指针、手动指针回退、
  pending 门禁和 rejected 门禁。归档源文件不变。
- 静态 staging 检查明确不调用 `check_bundle`、`launch_with`、本地安装、
  注册表或快捷方式 API。输出 `runtime_health_verified=false`、
  `restart_verified=false`，不能据此宣布安装、启动或自动健康回滚通过。
- `--offline`、`--private` 和 `scripts/check-rust-update.mjs` 的既有完整生命周期
  检查会启动程序，不属于本轮可自动执行范围。`fuze-update --verify-package`
  也会调用健康检查子进程，不能当成纯解包命令使用。

## 本轮实测记录

- 定向测试去重后 23 项通过：安装契约、指针回退、传输 / 重定向、续传、取消与差分。
  新用例用不可执行文本构造两包，确认不启动程序，并拒绝已有目录、损坏哈希、
  清单 / 包内构建身份不一致。开发验证器编译及 Rust 格式检查通过。
- 当前公开 FUZ 经独立 HTTPS 下载与 GitHub SHA-256 核对通过；此下载使用 PowerShell，
  只用于静态包验收，不能代替 Rust 生产传输结论。
- 两个真实包的 SHA-512、生产解包、build.json revision / sequence、Slint 入口选择、
  pending 指针 / 重复应用门禁、回退指针及 rejected 门禁全部通过。
  回退后独立测试目录的 current 为 `56ca08c45a63`、pending=false、
  rejected=`1c2281940730`。没有调用健康检查或程序启动。
- 目标包 `fuze-ui-lab.exe` 与 `fuze-update.exe` 的 Authenticode 状态均为 NotSigned。

| 最新公开资产在线检查 | 本轮结果 |
| --- | --- |
| 无令牌频道检查与 Range | 通过，目标始终为 `1c2281940730`，收到经过 Content-Range 校验的 206 响应 |
| 旧包差分重建 | **未通过**：600 秒调用预算到达，已收到 24,003,734 字节 Range 响应体，返回 `update-cancelled`；未完成整包校验，没有改变安装指针 |
| 强制全量回退 | **未执行**：本工具先要求差分成功，再以损坏旧 map 触发全量回退；前一步超时后停止。不能用续传成功替代这一场景 |
| 两进程断点续传 | 通过：第一进程接收并保留 1,048,576 字节后主动中断；第二进程 offset=1,048,576，接收 67,114,788 字节，496,704 ms 内完成整包 SHA-512，完成后 retained_after=0 |
| 实际安装 / 应用启动 / 自动健康回滚 | 本轮未执行，仍需独立验收 |

差分与续传两条验收进程在本轮部分重叠运行，上述耗时不作为串行性能基准。
观察到部分 1 MiB Range 响应体耗时二十多秒；另行获取完整公开 FUZ 并核对摘要约 6 秒。
这说明仍需定位生产 Range 传输耗时，但不能据此断言网络、代理、服务器或 WinHTTP
配置中的某一项是根因。没有通过修改超时、跳过散列或改用 PowerShell 来宣称生产更新通过。

**下一步开发侧工作**：隔离目录内做串行复测并定位 Range 响应体耗时，取得可复现证据后
再作最小修复；随后补齐该同版本的差分重建和强制全量回退。此项无需玩家操作游戏，
不应混入下面需要用户安排的实机清单。

## 需要用户安排的实机步骤

以下尚未执行。应先明确测试电脑、独立 Windows 测试用户或虚拟机、可保留的旧安装，
再由用户安排启动与版本切换；不要破坏日常安装来制造故障。

| 场景 | 实机验收标准 |
| --- | --- |
| 旧版正常在线更新 | 从真实 `56ca08c45a63` 受管安装入口检查公开频道，确认下载与安装；保存旧版，核对 `1c2281940730` 身份、UI 就绪和 pending 清除 |
| 无差分基线 | 便携包 / 首次本地安装可能没有缓存的 FUZ 与 blockmap，应走完整包；不能把这种情况当差分故障 |
| 下载中断与重开 | 在确认的测试安装中中断下载，再从实际更新入口继续；校验通过前不改 current；取消不触发全量重试 |
| 健康检查失败 | 仅在测试环境的目标版本副本制造可恢复的模型缺失，确认旧版自动恢复、rejected 记录且旧版能启动 |
| 进程启动 / 就绪失败 | 独立测试目标程序启动失败及无法报告 UI 就绪的场景；确认旧版实际启动、之后再次打开正常 |
| 正常再启动与手动回退 | 成功升级后退出再启动；再验证手动回退和回退后的再启动，区分入口指向、实际运行版本与设置保留 |
| 设置、历史与反馈 | 升级 / 回退前后确认设置、战绩、反馈队列保留。PR #105 尚未进入公开包；未来包含它的候选包需单独验证旧二进制读取新增日志 / 队列字段的降级边界 |
| 最终同版本环境 | 连接、首轮识别、短暂消失恢复、英雄详情、弹窗、IME、混合 DPI、真实焦点 / 最小化 / 跨屏行为需要同一最终候选包实测 |

上述场景不能由本轮 staging 回退或历史 CI 隐藏启动结果替代。用户没有提供本轮
脱敏游戏反馈，因此真实连接 / 对局问题继续标为未验证，不猜测其原因。

## 正式版边界

PR #105 已合入 master，反馈构建身份修复仍未打入 `1c2281940730`。
这一公开包的更新验证也不能替代未来包含反馈修复的候选包验收。
正式签名安装器、运行库依赖处理、上述真实更新 / 回滚及同版本环境验收仍需闭环。
主显示器识别的边界继续适用；可靠领取轮次仍未知，自动推荐保留整体胜率。
现阶段不能宣布 Rust 正式稳定版可发布；v0.6.3 稳定频道保留。
