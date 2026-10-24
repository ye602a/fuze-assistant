# 对局上传接入条件

核对日期：2026-10-24。本页只讨论向数据站提交对局，与[公开数据读取](client-api-strategy.md)分开。
本轮未读取个人战绩、申请上传会话或发送比赛样本。

## 已确认

依据站方[上传说明](https://github.com/valkia/aramgg_client/blob/master/docs/MATCH_HISTORY_UPLOAD_API.md)及[客户端源码说明](https://github.com/valkia/aramgg_client)：

- 配置仍从 `data.dtodo.cn/api/client/v1/config` 读取；写入使用独立的 `https://aramgg.com`。
- `POST /api/client/v1/match-history/upload-session` 申请短期会话，携带协议版本、客户端版本、区服和随机安装 UUID。
- `POST /api/client/v1/match-history/batches` 使用该会话上传。个人读取 API Key 不参与这两步。
- 只接受已结束的 `KIWI / queue 2400 / map 12` 对局，每批最多 20 场，并遵循服务端的大小和速率限制。
- 对局按 `platformId + gameId` 去重；客户端只有收到对应 `sourceKey + idempotencyKey` 的确认回执，才能标记成功。
- 样本包含参赛者的 PUUID、Riot ID 与战绩；不能在公开仓库、日志或错误消息中记录这些正文。
- 上游客户端要求正式打包且使用官方发布通道。源码运行、普通 fork、本地打包默认不能向生产环境提交。

上游的官方通道标记是发布防误操作条件，不是 FUZE 的授权凭据。不能复制这个标记，将 FUZE 冒充为站方官方客户端。

## FUZE 仍需站方确认

1. 是否接受 FUZE 作为第三方客户端提交**本人参加的已完成海克斯对局**。
2. 使用哪个应用标识与客户端版本，是否需要登记、白名单或额外凭据，以及允许的正式上传地址。
3. FUZE 应遵循的采集补丁、速率与样本字段要求；是否允许提交在接入获准前保存的样本。

公开文档没有给出 FUZE 的接入回执，也没有说明申请读取 Key 就能获得上传资格。
当前只能确认协议和上游发布规则，不能确认服务端已接受 FUZE。

## 本次 0.7.1 范围

本次 PR 不增加上传调用或赛后采集。已有本地队列、去重与模拟协议保留在独立的 [PR #127](https://github.com/ye602a/fuze-assistant/pull/127)，不随这次数据更新改动合入。
后续按站方答复推进真实上传，届时说明发送字段并让用户控制开关；不把模拟结果当成真实上传成功，也不自动补传此前积累的数据。
