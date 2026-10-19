# Rust 正式安装、签名与运行库验收

## 后续接线状态

2026-10-20：PR #109 / #112 / #115 / #116 均已合入 master，最新源码为 `f6e5a3d2a3b6`。
ZIP 安装与便携启动已复用依赖引导，新原生更新器在包检查、激活和前端启动前增加
依赖门禁，并通过最终提交 CI；原生更新器自身仍依赖 CRT，不能代替缺库机器上的
先行安装引导。见 [VC++ 构建记录与引导](VC_RUNTIME_BOOTSTRAP.md)。

用户明确安排先完成隔离开发：签名服务尚未准备，实机验收稍后安排。因此正式签名安装器、
签名后的产物回执 / 发布者信任接线、真实安装 / 更新回滚 / 重启仍是独立未完成项。
公开包 `e941c7e124c7` 不包含上述后续源码，不能用当前检查器给旧包补标依赖版本。
该公开包的最新只读补验仍显示 4 个自有 EXE 未签名、最低 VC++ 版本未记录；
隔离下载与静态回退结果见 [公开更新验收](PUBLIC_UPDATE_ACCEPTANCE.md)。
下面保留 PR #108 当时的审计快照；当前实现链与证据边界见
[正式版链路复核](RELEASE_CHAIN_REVIEW_20261020.md)。

## PR #108 静态审计快照

日期：2026-10-19。本轮仅静态检查公开 `1c2281940730` 载荷及安装源码，
增加开发侧只读检查。未生成安装包、签名、安装运行库、启动或切换 FUZE。

## 当时的实现与缺口

| 项目 | 已核对的现状 | 仍缺少的正式交付条件 |
| --- | --- | --- |
| Rust 分发 | `package-rust-preview.mjs` 产生 ZIP，`package-rust-update.mjs` 产生 FUZ / blockmap / 清单；阶段 workflow 需显式触发 | 没有 Rust 正式安装器构建 / 签名 / 发布链；本轮不触发阶段打包 |
| 本地安装 | `Install-local.cmd` → `fuze-update --install-local` → `local_setup::install_with` → `check_bundle` → 开始菜单 / Windows 应用登记 | 已有受管安装逻辑可复用；不是带前置依赖处理的正式安装器。`check_bundle` 会启动宿主，当前操作边界下不能执行 |
| FUZE 签名 | 公开包 4 个自有 EXE 均为 `NotSigned` | 受信发布者身份、签名服务或证书的安全接入、时间戳、签名后独立验证，以及安装 / 更新侧发布者验证 |
| 第三方签名 | `onnxruntime.dll`、`DirectML.dll`、`dxcompiler.dll`、`dxil.dll` 的 Windows Authenticode 检查均为 `Valid` | 这些签名不是 FUZE 的签名；仍需独立维护第三方来源 / 信任策略，不能只凭任意有效签名放行 |
| 更新信任 | FUZ / blockmap 散列、固定来源和解包白名单已实现；安装回执用于文件所有权 / 冲突检查 | 散列与回执不是发布者身份；现有 Rust 安装路径没有 Authenticode 发布者验证门禁 |
| VC++ 依赖 | README 要求 x64 VC++；没有最低版本记录、安装前判定或 redist 引导 | 必须在依赖 VC++ 的原生入口运行前处理；根据本次构建与第三方组件的要求记录最低版本，不猜测 |
| Electron 稳定版参考 | 旧版是 electron-builder NSIS；`installer/installer.nsh` 处理安装目录；CI 禁止自动发现签名身份，配置关闭更新代码签名验证 | 这条旧链也不能作为已完成 Rust 正式签名的证据；本轮不修改稳定版 |

上述判断来自仓库源码与公开载荷，不通过枚举本机证书库、私钥、凭据或 CI secret
来推断用户有没有证书。仓库未接通签名不等于用户没有可用的签名服务。

## 公开包的实际依赖

输入为上一轮通过 SHA-512 / SHA-256 核对后，使用生产解包器放到隔离目录的载荷。
8 个原生文件均为 x64；普通导入与延迟导入列表逐个与 MSVC `dumpbin /DEPENDENTS`
交叉核对一致。读取 PE 字节，不调用 `LoadLibrary`，不运行任何包内程序。

| 入口 / 组件 | 检出的 VC++ v14 依赖 |
| --- | --- |
| `fuze-update.exe`、`fuze-rust-host.exe`、`fuze-ocr-worker.exe` | `VCRUNTIME140.dll` |
| Slint `fuze-ui-lab.exe` | `VCRUNTIME140.dll`、`MSVCP140.dll` |
| `onnxruntime.dll` | 上述组件及 `VCRUNTIME140_1.dll`、`MSVCP140_1.dll`、`MSVCP140_ATOMIC_WAIT.dll` |

ONNX 另有 `DirectML.dll`、`d3d12.dll`、`dxgi.dll` 的延迟导入；四个第三方 DLL 均纳入
固定检查范围，即使某个组件通过动态加载使用，也不会仅检查 EXE 后漏掉它。
PE 导入枚举仍不能证明所有动态加载、API set、驱动、Windows 版本或 GPU 路径可用。

**`build.json.runtime = 1.26.0` 是 ONNX Runtime 版本，不是 VC++ 版本。**
公开构建没有声明可据以验收的最低 VC++ 版本；不能从 DLL 名、当前开发机版本或
不完整的链接器版本反推。微软要求匹配应用架构，且运行库不得早于生成应用的
MSVC 工具链；还需核对随包第三方组件的要求。
见 [VC++ 官方版本与架构要求](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)。

本机只读检测到上述 5 个实际需要的 DLL 均为 x64、版本 `14.50.35719.0`。
这只能说明本机文件现状；由于候选包未声明最低版本，报告保持
`meetsDeclaredMinimum=null`，不能据此认定缺少运行库的电脑安装通过。

## 可重复的只读检查

```text
node scripts/check-rust-release-readiness.mjs --bundle EXTRACTED_DIRECTORY --report NEW_REPORT.json
node --test tests/unit/rust-release-readiness.test.mjs
```

开发命令需 Windows x64 和仓库所用 Node，不是面向玩家的安装入口。
仅读取显式目录中的 `build.json` 和 8 个固定原生文件，以及系统目录中固定 VC DLL
的存在性、版本和 PE 架构。Windows PowerShell 用于 Authenticode 元数据检查；
不枚举证书库、不打印证书主体、个人路径或环境变量，不降低执行策略或系统防护。
系统签名验证可能进行证书链 / 吊销状态查询，不能将网络不可达解释成签名有效。

- 可选 `--publisher-sha256` 是预先认可的发布者证书 SHA-256，不能从待验收文件自己
  自动选择；可选 `--minimum-vc-runtime` 是有构建依据的 `14.x.x.x`，不能直接使用本文
  开发机读数。未提供时对应项保持阻断。
- 检查文件大小、PE / RVA 边界、普通 / 延迟导入、架构、签名状态、自有程序发布者匹配
  与时间戳、VC 依赖存在性和数值版本。前后读取的文件哈希必须一致。
- 报告仅写入包目录以外的新文件，不覆盖已有报告、包文件或安装指针。
- 退出码 `0` 仅表示所配置的静态检查通过；`2` 表示检查完成但仍有阻断；`1` 表示输入
  或检查失败。`releaseReady`、`installerVerified`、`runtimeLoadVerified` 和
  `installRollbackVerified` 始终为 false，不能将合成测试通过用作正式发布批准。
- 本轮公开包报告正确返回 `2`：自有 EXE 未签名 / 无时间戳、发布者约束未配置、最低
  VC 版本未记录。9 项测试通过，覆盖损坏 PE、延迟依赖、缺失 / 旧 / 错架构运行库、
  无效签名 / 发布者不匹配 / 时间戳缺失、读取期间文件变化、无效构建 / 策略输入，
  以及 Windows 下不可执行文件检查与输出保护。

这项工作补上开发侧证据采集与判定，**没有接入玩家安装流程，也没有实现运行库自动安装**。

## 后续源码与发布配置工作

1. 确定可验证的发布者身份和签名服务接入；私钥和认证留在受限签名环境，不放入仓库
   或玩家包。签名采用明确摘要和时间戳，独立检查有效链与预期发布者；最终产物使用
   `signtool verify /pa /all /tw` 等校验，不能仅凭签名操作返回成功。
   参考 [SignTool 官方说明](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool)。
2. 记录候选构建实际 MSVC 工具链和第三方组件要求，声明 x64 VC 最低版本；保留现有
   受管安装逻辑，补齐在 FUZE EXE 之前可运行的依赖引导。官方 VC redist 在独立步骤
   校验来源、有效 Microsoft 签名和版本，处理用户拒绝提权、安装失败和需重启；不要
   覆盖更高版本，不自动重启，不在卸载 FUZE 时卸载共享 VC 运行库。
   参考 [微软部署与安装选项](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files)。
3. 自有二进制签名完成后再生成 FUZ / blockmap / 清单和最终安装器；任何签名后变动都会
   改变摘要。安装器签名与 Rust 更新侧发布者验证仍需接线；现有未签名预览的升级入口
   必须单独处理，不能通过放宽正式信任规则假装完成迁移。

本轮没有安全可用的已签名候选安装器，也没有经记录确认的发布者 / VC 最低版本配置。
因此未自建自签名证书、未选购服务、未借用第三方签名，也未生成一个未经签名的 EXE
并称它为正式安装器。

## 需要用户安排的实机步骤

以下在最终同版本、已签名候选可用后，安排独立 Windows 用户或虚拟机执行，保留日常安装。

| 场景 | 验收要求 |
| --- | --- |
| 缺失运行库、仅有 x86、过旧 x64 | 前置检测先于 FUZE EXE，正确提出依赖需求；用户取消 / 拒绝 UAC 时不改当前安装 |
| 已有足够新版本 | 跳过 redist 安装，不降级或要求无意义的重启；仍验证最终程序实际启动 |
| 依赖安装失败或需重启 | 保留旧安装，明确失败 / 重启待处理状态，不自动重启；用户重启后可继续 |
| 签名与发布者 | 安装器、自有程序、第三方来源分别验证；损坏或错误发布者的载荷在激活前拒绝 |
| 首次安装 / 升级 / 卸载 | 标准用户权限、开始菜单和已安装应用登记；卸载不移除共享 VC 运行库或个人文件 |
| 更新失败 / 回滚 / 再启动 | 使用真实旧更新器取得最终候选，健康 / 启动失败能回到旧版，实际再次启动及数据保留 |

本机文件检测、静态 PE / 签名检查、模拟测试与 CI 均不替代以上实机结果。
IME、混合 DPI、真实窗口与同版本游戏场景继续按 [当前交接](../../CODEX_HANDOFF.md)
和 [公开更新验收](PUBLIC_UPDATE_ACCEPTANCE.md) 分别验收，Rust 尚不能作为完整稳定版放行。
