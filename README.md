<p align="center"><img src="docs/assets/fuze-banner.png" alt="符择 FUZE — 让每一次强化，都有依据" width="100%" /></p>

<h1 align="center">符择 FUZE</h1>
<p align="center"><strong>选英雄、看强化、导入出装。</strong></p>
<p align="center">海克斯大乱斗助手 · Rust 原生开发 · Windows x64 · 简体中文</p>
<p align="center"><a href="https://github.com/ye602a/fuze-assistant/releases/tag/rust-v0.7.0"><strong>下载 Windows 版</strong></a>　·　<a href="docs/PLAYER_GUIDE.md">快速开始</a>　·　<a href="https://github.com/ye602a/fuze-assistant/issues">问题反馈</a></p>

当前正式版：**v0.7.0**。下载 `FUZE-Setup-0.7.0-windows-x64.exe`，选择目录安装即可，所需 VC++ DLL 已包含在程序文件夹中。安装包暂未签名；也可选择发布页的 ZIP 便携包。

FUZE 是 Windows 上的海克斯大乱斗助手。选人时查看阵容和候选英雄，局内识别三张强化卡片，选好的装备方案可以直接导入游戏商店。

## Rust 开发，优先控制内存和 CPU 占用

核心服务使用 Rust，主界面使用 Slint，三卡浮窗使用 Direct2D / DirectWrite。程序按对局状态安排识别，复用未变化画面的结果，并优先读取本地数据，减少重复下载和推理。

内存和 CPU 占用会受识别后端、分辨率和对局阶段影响；目前没有统一测试条件下的占用数值可供比较。

## 功能

### 强化识别

按游戏里的左、中、右卡位展示强化名称、胜率和选取率。未识别的卡位保留空位；缺少对应版本的统计时显示“暂无统计”。

自动推荐使用整体胜率；阶段胜率可单独参考，暂不自动推断你已领取了第几次强化。

### 选人参考

上方显示我方五人阵容，下方集中展示候选英雄。点击英雄可查看胜率、技能和出装。

### 装备方案导入

选人时切回自己的英雄，选择出装路线，点击「导入路线到游戏商店」。进入游戏后，打开商店的装备方案，选择对应的 FUZE 方案。

方案按阶段列出装备。导入不会自动购买，也不会替换你的个人方案；请在装备方案中查看，游戏内置的“推荐”页不会因此改变。

### 战绩与问题反馈

可查看个人战绩和已保存的识别记录。遇到问题时，在程序内导出脱敏反馈，再到 Issues 附上文件和现象说明。

## 开始使用

1. 从发布页下载 `FUZE-Setup-0.7.0-windows-x64.exe`，选择目录完成安装。
2. 登录英雄联盟并打开符择，确认首页显示连接已验证。
3. 进入海克斯大乱斗。选人时查看阵容和装备方案，强化出现时查看三卡参考。

玩家无需申请 API Key、配置令牌或安装开发工具。GitHub 自动生成的 Source code 压缩包是源码，请下载 Setup 或程序 ZIP。

## 数据与使用范围

- 强化画面在本机完成 OCR 处理，不自动上传截图。
- 不自动选英雄、选强化或购买装备。装备导入由你确认；自动导入需要单独开启。
- 推荐数据通过现有公开接口检查更新，优先使用本地缓存；程序更新需手动确认。
- 反馈可手动导出。自动 GitHub 反馈需要独立授权，开启后报告会公开在 Issues。

当前支持简体中文 Windows x64，游戏画面识别以主显示器为准。推荐数据来自 ARAMGG 等上游来源；数据版本可能落后于游戏版本，缺少新统计时不会套用旧版胜率。详情见[玩家指南](docs/PLAYER_GUIDE.md)与[来源说明](THANKS.md)。

---

<p align="center"><a href="docs/development/README.md">开发资料</a> · <a href="THANKS.md">致谢与第三方说明</a> · <a href="https://github.com/ye602a/fuze-assistant/releases">版本记录</a></p>
