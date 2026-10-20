# 未签名独立安装器与数据上传核对

## 安装器

目标发行 v0.7.1。`rust-prototypes.yml` 的显式阶段验收将构建 Slint 包、
`FUZE-Setup-0.7.1-windows-x64.exe`、校验文件与构建回执；普通 PR 不发布。
安装器用 Inno Setup 作为临时解包与向导容器，不引入第二套安装目录、注册表或卸载逻辑。

链路：Setup → `rust-setup-bootstrap.ps1` → 原包 `Install-local.ps1` 中的
`Read-VcContract` / `Get-VcInventory` / `Invoke-VcFlow` / `Get-VcStartInfo`
→ `fuze-update --install-local-confirmed` → 原 `local_setup` / `install_entries` /
`app_registration`。确认来自安装向导按钮；原 ZIP 的交互确认入口继续保留。
缺库时显式打开固定微软下载来源，用户选择文件、再次确认后才核验散列 / 签名 / 版本并请求 UAC。
静默模式缺库即失败，不下载或安装运行库。任何重启要求都停止 FUZE 安装，由用户自行安排。

安装器先只读核对 PowerShell 各 scope；显式 `Restricted` / `AllSigned` 直接阻止安装。
未配置限制的 Windows 客户端默认策略不允许脚本文件，因此仅对随包辅助脚本的子进程指定
`RemoteSigned`，不调用 `Set-ExecutionPolicy`，不改变系统 / 用户持久配置，不越过组策略。
CI 通过继承的进程级 Restricted 验证必须失败且不能创建安装目录 / 应用登记；
没有在用户机器修改策略。仅支持原生 Windows x64，不宣称 ARM64 运行库兼容。

安装完成不启动 FUZE。首次启动仍使用原健康确认与回滚协议。卸载仍由原生维护程序处理，
保留设置、记录、凭据与未知文件。`--uninstall-local-confirmed` 供显式无人值守调用，
仍要求从受管目录之外运行，并保留运行中检查、安装锁、所有权回执和注册表校验。

构建回执分别记录完整安装器源码提交、包内 revision、安装器散列、Inno 版本、VC++ 基线及
原生文件散列。不能把 Setup 版本当成历史日志的运行版本。未签名是明确发行属性。
阶段 CI 实际运行 Setup.exe，验证首次安装、同版本重复安装、原生卸载及数据保留；
已有更新验证器继续验证健康失败 / 进程启动失败回滚与再次启动。CI 不替代玩家电脑验收。

## 对战数据贡献是否实装

2026-10-20 只读核对结果：**当前 Rust 版未实装**。

- `post_game_history.rs` 是赛后延迟刷新本地战绩，明确不上传。
- Rust 的 `fuze-lcu/history.rs` / transport 读取 LCU 和 SGP 战绩，不申请数据站上传会话。
- Rust 网络上传入口 `github_feedback.rs` 发送允许列表中的诊断到 GitHub Issues；
  与网站对战上传不是同一功能，不能当作已完成迁移。
- 保留的 TypeScript `services/match-history/upload/` 有会话、outbox、幂等回执及重试实现，
  但它不会随 Rust 运行。其 production gate 要求 packaged + official；当前旧版 workflow
  使用 `fuze-private` 和 localhost，因此也不能凭旧代码存在宣称 FUZE 正在生产上传。

公开 `GET https://data.dtodo.cn/api/client/v1/config` 本次返回 dataVersion `16.20.3`，
`matchHistoryUpload.cloudflareEnabled=true`、`enabled=false`，会话路径
`/api/client/v1/match-history/upload-session`、批次路径
`/api/client/v1/match-history/batches`，最大 20 场、目标补丁 16.20。
这是公开配置证据，不等于 POST 已验证可用、网站授权 FUZE 接入或本客户端已上传成功。
没有创建上传会话、访问游戏或读取 / 上传个人对局。

仓库历史协议 [MATCH_HISTORY_UPLOAD_API.md](../../MATCH_HISTORY_UPLOAD_API.md) 指向
独立写入 Origin `https://aramgg.com`，且载荷包括 PUUID / Riot ID。这不能直接套用于“匿名贡献”，
也不能把读取数据的域名当成写入 Origin。后续接入需要核对站点当前协议与身份字段要求，
设计默认关闭的单独用户授权、最小字段白名单、持久化幂等 outbox、回执及有界重试；
先使用合成对局与隔离服务验收，再安排经授权的真实上传。

共用公开数据接口不证明网站归属或底层数据所有权。历史代码来源、当前读取接口、可用写入接口
和实际启用的上传链路分别记录，本轮不改变数据贡献开关。
