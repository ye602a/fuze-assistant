# 复用资源

- overview/history/settings/network/capture/recognition/data/rune SVG 的路径直接来自 `v0.6.3:src/renderer/components/RuneGlyph.vue`，保留原有图标体系；diamond 是同版状态菱形的矢量表达。
- 品牌继续引用仓库 `public/fuze.svg`，不创建另一个品牌。
- champion PNG 与三个 augment PNG 取自此前已缓存的公开数据图标，原 URL 为 `https://cdn.dtodo.cn/hextech/champion-icons/<id>.png` 和 `https://cdn.dtodo.cn/hextech/augment-icons/<name>.png`。对应数据快照为仓库的 `resources/client-data/versions/16.20.1`。
- 图片仅用于隔离的固定界面预览，不代表本机英雄、个人战绩或实际领取；本轮没有下载新资源。沿用仓库现有资源归属要求，不能因迁移 Rust 宣称这些游戏美术为自行创作。

- `public-cache/hextech/` 复用同一批公共缓存中的装备、技能、英雄和强化图标；`src/public_art.rs` 只匹配固定 CDN 路径。重复的图像直接引用上级已有文件。按需在后台解码，缺失图标不冒充其他图标，不读取用户图片目录。
