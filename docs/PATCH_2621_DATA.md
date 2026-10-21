# 26.21 符文资料与识别适配

## 来源与范围

- 官方范围：[Riot 26.21 patch notes](https://www.leagueoflegends.com/en-us/news/game-updates/league-of-legends-patch-26-21-notes/)，35 新增、13 退池。
- 名称与图标：[16.21 简中客户端导出](https://raw.communitydragon.org/16.21/plugins/rcp-be-lol-game-data/global/zh_cn/v1/cherry-augments.json)。
- 模式池：[KIWI 客户端模式数据](https://raw.communitydragon.org/16.21/game/maps/modespecificdata/kiwi.bin.json)。
- 说明：[zh_cn 客户端字符串表](https://raw.communitydragon.org/16.21/game/zh_cn/data/menu/en_us/lol.stringtable.json)。路径中的 `en_us` 是导出路径，内容来自 `zh_cn` 区域。

CommunityDragon 是客户端资源导出源，不是国服统计服务，也不构成实际国服画面验收。
KIWI 的 249 条 AugmentData 排除 `GoH_PentakillAug_` 的 5 个特殊技能选择后为 244 条。
名称交叉核对字符串表和简中目录，图标沿用同版本客户端路径。不是人工翻译英文名称。
说明中的字符串引用递归展开；客户端运行时数值变量保留为 `?`，不填入旧版数值。

## 统计与版本行为

截至本轮查询，`data.dtodo.cn/api/client/v1/config` 仍指向 16.20.4。
内置资料只补名称、说明、图标和稀有度，不含胜率、选取率或推荐分数。
当前简中视图对不高于 16.21 的来源应用本次清单，并单独标记资料身份
`zh-CN@<原统计版本>-augments16.21`；原统计版本、下载指针和更新格式不变。
英雄符文统计须满足来源版本及 `stats.gamePatch` 对应 16.21；如有
`winRateGamePatch`，也必须对应。缺少或冲突即清除整组符文统计（包含 rank/tier），
不能借用旧排名继续推荐。浮窗使用现有“暂无统计”状态。

只保留当前池内的英雄符文行。新增符文无英雄统计时，浮窗使用基础资料卡，
不虚构其英雄胜率；英雄详情不会伪造新增的英雄统计记录。
历史与显式旧版准备路径不套用此覆盖；更高版本不降回 16.21。
不修改等级门禁和选择画面检测阈值，保留标题完整匹配边界，并明确拦截已知商店装备
“海克斯科技核心”，避免模糊匹配到“海克斯科技龙魂”。

## 复现导入

把上面三份固定版本 JSON 分别保存为 `cherry-augments-zh_cn.json`、`kiwi.json`、
`strings.json`，置于隔离目录。仅导入脚本需要 Python 的 `xxhash` 模块，应用无新增依赖。
字符串表键使用小写键名的 XXH3-64 低 38 位。

```text
python scripts/import-augments-16.21.py <源文件目录> resources/augments-16.21-zh-CN.json
```

输出文件包含三份原始文件的 SHA-256。原始大型导出不放入仓库。
图标复用有大小限制的后台下载/PNG 解码和本机缓存，固定 16.21 路径，不使用 `latest`。
本轮 244 个 URL 均返回 PNG；Windows HTTP 使用应用同名 User-Agent 抽查成功。
离线无图标缓存时仍不承诺首次显示图标。

## 针对性验证

- `cargo test --manifest-path rust/Cargo.toml -p fuze-data --locked --offline`：统计补丁、退池、旧版读取、详情缓存。
- `cargo test --manifest-path rust/Cargo.toml -p fuze-ocr title_matcher --locked --offline`：35 个新增中文名称与商店装备拒绝。
- `cargo test --manifest-path rust/Cargo.toml -p fuze-worker-host champion_assets::tests --locked --offline`：所有新资料图标路径、缓存及旧请求隔离。

以上是源码/数据/HTTP 检查，没有启动 FUZE 或游戏，尚未验证 26.21 国服实际卡片截图与完整识别链路。
