# Rust 数据贡献：模拟接入与网站规则

核对日期：2026-10-23。本轮没有请求上传会话、提交比赛、登录开发者后台或读取本机个人战绩。

## 当前实现

- 开关默认开启，在“数据与更新”明确说明采集范围、包含的身份字段和“暂不实际上传”；可关闭并跨重启保存。
- 复用已有赛后延迟读取，在同一次本人 SGP SUMMARY 响应中提取完整样本，不增加查战绩请求。
  启动、手动刷新、翻页和本地缓存浏览不会补采贡献样本，也不查询相遇玩家的战绩。
- 只整理已验证当前账号参加、区服一致、`KIWI / queue 2400 / map 12 / GameComplete` 的对局。
  本轮模拟采集补丁固定为已核对的 `16.21`，每轮最多最近 10 场合格样本。
  账号复核失败、旧补丁、未结束或只有简略 LCU 回退数据时不入队；缺失强化不从 OCR 或等级推断。
- 只保留上传规范允许的字段。完整样本包括该局所有参与者的 PUUID、Riot ID、英雄、装备、强化、KDA 和胜负；
  不保留响应原文、登录令牌、认证地址、截图和本地当前账号标识。
- `platformId + gameId` 标识比赛，规范化内容摘要标识版本，重试保持同一幂等键与原观察时间。
  成功回执必须匹配 `sourceKey + idempotencyKey`；已确认项去除正文并保留去重记录。
  永久拒收停止重试，可重试拒收、缺失回执、错误回执和网络失败仍保留待传；兼容 `game_archived` 可重试语义。
- 本地队列与旧战绩、设置格式分离；关闭后不加入新样本，已有待传数据保留。
  模拟阶段待传上限 200，达到上限明确提示且不覆盖旧队列。界面显示待传、已确认、未接受计数和本次结果。
- 协议函数只接受调用方提供的传输函数，当前应用没有 HTTP 调用点。测试用内存模拟会话与回执，真实上传计数不会伪造。

主要入口：`fuze-lcu/src/contribution.rs`（规范化），`fuze-worker-host/src/account_history.rs`（赛后复用），
`fuze-worker-host/src/data_contribution.rs`（开关、队列、模拟协议），`rust/ui-lab/ui/pages.slint`（提示与状态）。

## 谁可以上传

公开的 [上传说明](https://github.com/valkia/aramgg_client/blob/master/docs/MATCH_HISTORY_UPLOAD_API.md)
及 [OpenAPI](https://github.com/valkia/aramgg_client/blob/master/docs/api/match-history-upload.openapi.yaml)
把写入接口定位为官方客户端比赛采集。上游还要求正式打包、官方发布通道和内置生产写入地址；普通 fork 默认不能写生产服务。
这是上游客户端的防误上传门禁，不等于服务端凭此完成身份认证。

网站确实提供 [第三方开发者入口](https://aramgg.com/zh-CN/client)，但其公开说明是申请 Key **读取数据**。
截至本次核对，没有找到第三方客户端上传登记、贡献额度交换或允许任意第三方批量上传的公开规则。
不能把“有上传 URL”或“能申请读取 Key”解释为 FUZE 已获生产上传接入资格，也不将 FUZE 标成 ARAMGG 官方客户端。

## 上传协议与当前策略

| 内容 | 已核对规则 |
| --- | --- |
| 配置 | `GET https://data.dtodo.cn/api/client/v1/config` |
| 会话 | `POST https://aramgg.com/api/client/v1/match-history/upload-session`，v2、客户端版本、区服、随机 UUID v4 安装标识 |
| 批次 | `POST https://aramgg.com/api/client/v1/match-history/batches`，短期 Bearer 会话，不内置长期 Key |
| 限制 | 每批最多 20 场，还需遵守会话返回的大小上限；当前配置每轮最多 7 批 |
| 确认 | `inserted / duplicate / updated` 确认；`rejected` 按顶层 `retryable` 处理 |
| 错误 | 401 重新申请一次会话；413 缩批；429 遵循 Retry-After；网络和 5xx 有上限退避 |
| 范围 | 服务端不下发玩家采集任务，也不允许根据回执向外扩展玩家 |

当前公开配置是 `cloudflareEnabled=true`、`targetGamePatch=16.21`。
下载数据仍为 `gamePatch=16.20`、`dataVersion=16.20.4`，这是不同字段，不能因为已收集新补丁就声称新统计已发布。
上游配置允许额外采集相遇玩家；FUZE 不采用该范围，只使用本人参加的比赛。

发现一处文档差异：OpenAPI 的 `servers` 仍写 `data.dtodo.cn`，较新的上传说明明确写独立的 `aramgg.com` Origin。
本轮只使用路径做模拟，没有向任一域名发送 POST。正式接入前须由站方确认 FUZE 的接入资格、正式地址及允许的客户端版本标识。

## 其他公开入口

依据 [第三方读取 API 文档](https://data.dtodo.cn/api/v1/zh-CN/docs/cf-data-api.md)：

| 接口或入口 | 用途与访问规则 |
| --- | --- |
| `/api/v1/zh-CN/config.json`、`/docs/cf-data-api.md` | 配置和文档免费、无需 Key；文档路径位于该语言 API 前缀下 |
| `/augments.json`、`/champions.json`、`/items.json`、`/summoner-spells.json` | 名称、图标和列表；需 Key，每次成功 GET 1 credit |
| `/champions/{id}.json` | 展开的英雄、强化和出装详情；需 Key，2 credits |
| `/data/champion-augments/{id}.json`、`/data/augments-stats-raw.json`、其他现存 `/data/*` | 原始兼容数据；需 Key，1 credit；不是另一套免费通道 |
| `https://data.dtodo.cn/developer.html` | GitHub 登录后申请/管理 Key、查看额度；当前免费 200 credits/日、60 请求/分 |
| `/api/user/me`、`/api/user/api-keys`、`/api/user/usage` | 开发者门户使用的账号/Key/用量接口；需登录，不是比赛贡献接口 |
| `/api/auth/github/start`、`/api/billing/checkout` | 门户登录和付费流程；本轮只读公开前端路由，未触发 |

表中省略域名的业务文件均以 `https://data.dtodo.cn/api/v1/zh-CN` 为前缀，`/api/user/*` 等门户路由除外。
读取接口只支持 GET/HEAD；HEAD 仍需鉴权但不扣 credits。额度按 UTC 00:00（北京时间 08:00）重置。
不提供公开 changelog/补丁历史接口，更新随主站刷新、不承诺实时。

单英雄强化胜率使用上传样本，达到 255 场才展示；三强化组合达到 151 场才展示、最多 5 组。
缺失统计为 null，不应显示成 0% 或套用其他补丁。公开配置不能证明这些新统计已达到展示门槛。

## 此轮边界与验证

本轮完成本地接入与模拟协议，**没有启用真实发送**。下一步正式发送仍需接入动态采集策略、
随机安装标识持久化和 HTTP 会话/限流重试；不能将本轮模拟回执当成站方接受 FUZE 的证据。
生产启用前须再次确认告知内容与开关状态，不能静默发送本轮积累的数据。

针对合成 SGP 响应验证完整提取至模拟成功回执、本人/区服/结束/补丁门禁、字段排除、
去重、失败重试键、部分/错误回执、永久/可重试拒收、已确认正文移除以及开关持久化。
另检查原设置、赛后调度、战绩缓存和 UI 编译；未启动或切换本机 FUZE，未读取个人游戏数据。
