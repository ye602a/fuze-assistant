<p align="center"><img src="docs/assets/fuze-banner.png" alt="符择 FUZE — 让每一次强化，都有依据" width="100%" /></p>

<h1 align="center">符择 FUZE</h1>
<p align="center"><strong>看清三张强化，再做你的选择。</strong></p>
<p align="center">海克斯大乱斗助手 · Windows x64 · 简体中文 · 本地识别</p>
<p align="center"><a href="https://github.com/ye602a/fuze-assistant/releases/tag/rust-preview"><strong>下载 Rust 测试版</strong></a>　·　<a href="https://github.com/ye602a/fuze-assistant/releases/tag/v0.6.3">下载稳定版</a>　·　<a href="docs/PLAYER_GUIDE.md">使用指南</a>　·　<a href="https://github.com/ye602a/fuze-assistant/issues">问题反馈</a></p>

## 把注意力留给对局

符择连接英雄联盟客户端，在海克斯大乱斗中识别强化选择，按游戏里的左、中、右顺序展示名称与推荐数据。选择和刷新始终由你操作。

| 场景 | 你能看到什么 |
| :--- | :--- |
| **选人** | 我方五人阵容、按胜率排列的备选英雄，以及英雄详情、出装和技能参考 |
| **选择强化** | 对应三张卡位的推荐、胜率与选取率；未读到的位置明确提示，不用其他文字补位 |
| **查看状态** | 客户端连接、画面采集、中文识别和推荐数据版本，区分等待、工作与异常 |
| **赛后回看** | 本地识别观察、已获取的个人战绩，以及可导出的脱敏故障摘要 |

<p align="center"><img src="docs/assets/three-card-preview.png" alt="符择三卡界面示意，非当前 Rust 测试版实战截图" width="100%" /><br /><sub>既有离线界面示意，使用测试名称与数值；实际样式以下载版本为准。</sub></p>

## 选一个适合你的版本

| 版本 | 适合谁 | 下载什么 |
| :--- | :--- | :--- |
| [**Rust 测试版**](https://github.com/ye602a/fuze-assistant/releases/tag/rust-preview) | 体验当前开发版本，愿意反馈识别和兼容性问题 | `fuze-rust-preview-*.zip`，完整解压后运行 `Start-this-build.cmd` |
| [**v0.6.3 稳定版**](https://github.com/ye602a/fuze-assistant/releases/tag/v0.6.3) | 使用保留的 Electron 版本，或需要回退 | `Fuze-Setup-0.6.3.exe` |

仓库与下载均已公开，**下载和更新 Rust 测试版不需要 GitHub 令牌**。不要把 GitHub 自动生成的 Source code 压缩包当成可运行程序。

主分支可能领先于下载包。只有发布到对应频道的包才会成为可安装更新；不是每次提交或 PR 都会生成新版本。两个版本可以保留，但不要同时运行两套识别。

## 三步开始

1. 下载所选版本，安装或完整解压。Rust 测试版的详细说明见[测试指南](docs/RUST_PLAYER_TEST_GUIDE.txt)。
2. 登录英雄联盟并打开符择，确认首页显示连接已验证。自动发现失败时，再使用设置中的游戏目录回退。
3. 正常进入海克斯大乱斗，出现强化选择时查看推荐。无边框或窗口模式更便于排查采集兼容性。

大厅里“等待对局”、截图与识别待命是正常状态；“已发现客户端”并不等于已经通信成功。

## 本地优先，操作由你决定

- **本地 OCR**：画面在本机内存中处理，不上传游戏截图；不自动选择英雄或强化。
- **按需识别**：结合游戏阶段、等级和画面门禁减少无效工作，OCR 后端支持 CPU 与 DirectML 回退。
- **缓存优先**：已有数据优先本地展示，数据缺失或暂不可用时明确提示。
- **主动更新**：由你检查、下载和确认安装；条件满足时使用差分下载，失败可回退完整包。
- **可选反馈**：可以手动导出脱敏摘要；GitHub 自动反馈需另行授权并主动开启，报告会公开在 Issues。

## 你可能关心

**推荐已经按第几次领取切换了吗？**

自动排序仍使用整体胜率。Rust 测试版提供四次选择的阶段胜率参考；缺数据或样本不足会标明。等级、刷新次数和属性碎片都不能单独证明实际领取轮次。

**为什么英雄详情或战绩没有数据？**

数据覆盖、缓存完整性、区服识别和远端接口都可能影响结果。没有数据时不会套用其他英雄的数据；查询失败保留已有缓存。请附上版本、发生位置和脱敏反馈，见[问题反馈](docs/PLAYER_GUIDE.md#反馈问题)。

**下载免费公开，为什么自动反馈还需要令牌？**

公开下载是读取，创建 GitHub Issue 是写入，二者权限不同。不想配置授权时，直接导出反馈文件并手动提交即可；安装包不内置共享令牌。

---

<p align="center"><a href="docs/development/README.md">开发资料</a> · <a href="THANKS.md">致谢与第三方说明</a> · <a href="https://github.com/ye602a/fuze-assistant/releases">版本记录</a></p>
