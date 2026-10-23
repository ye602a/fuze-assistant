# FUZE 数据读取与更新

核对日期：2026-10-24。当前 Rust 继续使用已有公开数据接口，不读取个人 API Key，也不经过 FUZE 自建服务器。玩家无需申请开发者账号。

## 三条独立路径

| 功能 | 来源与入口 | 当前接线 |
| --- | --- | --- |
| 推荐数据更新 | `https://data.dtodo.cn/api/client/v1/config` 及其版本数据 | Rust 下载器，无鉴权头 |
| FUZE 程序更新 | `ye602a/fuze-assistant` 的 `rust-preview` Release | Rust 更新器，公开下载无需令牌 |
| 对局贡献 | 站方独立的上传会话与批次接口 | 本次未接入，条件见[对局上传接入核对](DATA_CONTRIBUTION_ACCESS.md) |

数据配置中的 `client.latestVersion`、`downloadUrl`、`updateFeedUrl` 是上游客户端的信息，不能用于 FUZE 程序更新。旧 Electron 的 `latest.yml` 也不是 Rust 更新清单。

## Rust 数据链路

1. 先展示随包数据或完整本地缓存。
2. 空闲时检查数据版本；手动“检查数据更新”只处理简体中文。
3. 本地已有同版或更新的完整资料时，不重复下载。
4. 有新版本时下载清单和所需文件，显示实际已完成文件数；校验通过后启用。
5. 失败或取消时继续使用现有数据，结果保留在数据卡片中。

入口位于 `rust/crates/fuze-worker-host/src/data_update.rs`、`background_data.rs` 和 `update_io.rs::data_get`。
构建时的 `scripts/prepare-rust-data.mjs` 复用 `fuze-rust-host --prepare-data`，保留打包所需的语言数据；应用内的手动简中更新不会触发它。

资料补丁与统计补丁分别显示。2026-10-24 核对时，公开配置仍为 `dataVersion=16.20.4`、`gamePatch=16.20`，内置符文资料为 16.21。接口可访问或数据检查完成，都不表示新补丁胜率已发布。

## 保留与清理

- 保留公开数据接口、现有缓存格式、程序更新清单及旧设置迁移。
- 旧 JS 数据下载器移入 `legacy/electron/scripts/`，复用归档内的路径校验模块；删除根目录的重复校验副本。
- 根目录 `npm run prepare:client-data` 统一调用 Rust 数据准备脚本。
- [旧 Electron 接口说明](../legacy/electron/docs/client-api-strategy.md)仅供追溯，不作为当前 Rust 接入指南。

站方另有带 Key 的 `/api/v1/zh-CN/*` 开发者读取接口。本次不接入、不分发 Key、不添加代理服务；它不授予对局上传权限。
站方将客户端分发与第三方开发者 API 分开说明，公开接口能匿名读取不等于对未来可用性或第三方使用范围的承诺。来源：[站方接口分发说明](https://github.com/valkia/aramgg_client/blob/master/docs/client-api-strategy.md)。
