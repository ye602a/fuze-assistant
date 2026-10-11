<p align="center"><img src="docs/assets/fuze-banner.png" alt="符择 Fuze — 让每一次强化，都有依据" width="100%" /></p>

<h1 align="center">符择 Fuze</h1>
<p align="center"><strong>看清三张强化，再做你的选择。</strong></p>
<p align="center">海克斯大乱斗助手 · Windows · 中文识别 · 本地优先</p>
<p align="center"><a href="https://github.com/ye602a/fuze-assistant/releases/latest"><strong>下载最新版</strong></a>　·　<a href="docs/PLAYER_GUIDE.md">使用指南</a>　·　<a href="https://github.com/ye602a/fuze-assistant/releases">更新记录</a>　·　<a href="README.en.md">English</a></p>

---

## 把注意力留给对局

符择自动连接英雄联盟客户端，在海克斯大乱斗中识别三张强化，将推荐信息放在对应卡位旁。选哪个、何时重选，都由你决定。

| 什么时候 | 符择能帮你做什么 |
| :--- | :--- |
| **选人时** | 查看当前英雄、备选英雄、出装与技能参考 |
| **选择强化时** | 按左、中、右展示三卡推荐，不打乱游戏中的位置 |
| **对局结束后** | 回看本机保存的候选卡记录与可用战绩 |
| **平时** | 查看连接状态，调整浮窗、识别和本地存储设置 |

<p align="center"><img src="docs/assets/three-card-preview.png" alt="符择三卡推荐界面示例" width="100%" /><br /><sub>离线界面示例，使用测试名称与数值。统计仅供参考，不能保证单局效果。</sub></p>

## 三步开始

1. 从 [Releases](https://github.com/ye602a/fuze-assistant/releases/latest) 下载并安装 `Fuze-Setup-版本号.exe`。
2. 打开符择，启动并登录英雄联盟，确认主页显示实际连接状态。
3. 正常进入海克斯大乱斗；出现强化选择后，查看对应位置的推荐。

**当前仓库为私有测试项目，下载需要仓库访问权限。** 推荐使用无边框或窗口模式；远程桌面和不同显卡环境可能需要单独验证。客户端进程被发现不等于连接已经成功，遇到问题先看 [使用指南](docs/PLAYER_GUIDE.md)。

## 轻一些，也可控一些

- **本机识别**：不上传游戏截图，不自动替你选择强化。
- **按需工作**：结合等级与画面变化减少重复识别；GPU 不可用时可回退 CPU。
- **缓存优先**：已有推荐数据可从本地读取；数据缺失时明确提示。
- **手动更新**：由你检查、下载和确认安装，选人及对局期间不安装更新。

## 常见问题

**国服战绩查询失败怎么办？** 远端接口可能不可用。符择会尝试客户端近期记录；已保存的本人战绩和本地选择记录仍可回看，但不能补出从未取得的数据。

**会自动使用第几轮的胜率吗？** 目前三卡使用整体统计，不按等级猜测实际领取轮次。各安装包具体包含哪些功能，以 [正式更新记录](https://github.com/ye602a/fuze-assistant/releases) 为准；主分支可能领先于已发布版本。

**能直接在软件里更新吗？** 可以在设置中检查更新。私有频道需要本机授权；也可以从 Releases 手动下载。详见 [安装与更新](docs/PLAYER_GUIDE.md#私有频道更新)。

---

<p align="center"><a href="docs/development/README.md">开发文档</a> · <a href="THANKS.md">致谢与第三方说明</a></p>
