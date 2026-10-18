# 复用资源

- 通用 UI 图标统一使用 Lucide 0.468.0（提交 `f12b0de177fbc2a6795e99be065887e72b237123`），24×24、2px 描边、圆角端点，原始路径来自 https://github.com/lucide-icons/lucide/tree/0.468.0/icons 。只将 `currentColor` 改为蓝金界面的基色，由 Slint 按状态着色。
- overview/layout-dashboard、history/history、settings/settings、network/plug-zap、capture/scan-line、recognition/scan-text、data/database、rune/gem；窗口和反馈控制使用 x、minus、chevron-down、info。`diamond.svg` 是保留的品牌状态点，不承载按钮含义。
- 许可证见 `LUCIDE-LICENSE.txt`，同份声明加入随包 `THANKS.md`。游戏图标和品牌没有替换为 UI 图标。
- 品牌继续引用仓库 `public/fuze.svg`，不创建另一个品牌。
- champion PNG 与三个 augment PNG 取自此前已缓存的公开数据图标，原 URL 为 `https://cdn.dtodo.cn/hextech/champion-icons/<id>.png` 和 `https://cdn.dtodo.cn/hextech/augment-icons/<name>.png`。对应数据快照为仓库的 `resources/client-data/versions/16.20.1`。
- 图片用于固定界面预览与生产界面展示，不代表本机英雄、个人战绩或实际领取。沿用仓库现有资源归属要求，不能因迁移 Rust 宣称这些游戏美术为自行创作。

- `public-cache/hextech/` 复用同一批公共缓存中的装备、技能、英雄和强化图标；`src/public_art.rs` 只匹配固定 CDN 路径。重复的图像直接引用上级已有文件。按需在后台解码，缺失图标不冒充其他图标，不读取用户图片目录。
- 2026-10-17：依据随包 16.20.1 数据中的公开 CDN 地址补齐 146 张强化图标，覆盖当前数据的 184 个不同图标。`scripts/prepare-rust-augment-icons.mjs --download` 显式更新，默认调用只检查本地资源；生成索引为 `src/public_augment_art.rs`。构建不会联网下载图片。上述游戏图标的原有权利归属不变。
