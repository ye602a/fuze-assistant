# Fuze 0.4.1 依赖与性能检查

检查日期：2026-10-11（Asia/Hong_Kong）。本文记录当前个人分支；`PERFORMANCE_DIAGNOSTICS.md` 中 2026 年 7–8 月的历史数据不作为本次效果证明。

## 11 项告警的处理

“11 项”是 npm 按受影响依赖节点统计的数量，其中多项由同一底层问题向上传播。

| 节点 | 等级 | 原因与处理 |
| --- | --- | --- |
| vue、@vue/server-renderer | 高 × 2 | Vue 3.5.34 带入服务端渲染 XSS；升级同版本族至 3.5.43。应用本身不使用服务端渲染，但不能据此忽略依赖修复。 |
| brace-expansion | 高 × 1 | ESLint 的 minimatch 10 分支含拒绝服务问题；仅将该分支升级到 5.0.12，保留其他调用方需要的 1.x/2.x API。 |
| sprintf-js、roarr、global-agent、@electron/get、app-builder-lib、dmg-builder、electron-builder-squirrel-windows、electron-builder | 中 × 8 | 打包下载工具的旧日志链带入无修复版的 sprintf-js。将 app-builder-lib 下的 @electron/get 升至 5.1.0，移除该旧日志链；不采用审计工具建议的整体降级。 |

参考：[Vue 漏洞](https://github.com/advisories/GHSA-g2v6-rqmx-r4w6)、[sprintf-js 漏洞](https://github.com/advisories/GHSA-hp3w-g68c-fv3c)。

最终 `npm audit` 与 `npm audit --omit=dev` 均为 0 项。零告警仅表示当前依赖数据库未报告已知问题，不等于软件不存在缺陷。Vue 等 renderer 依赖即使归在 devDependencies，也会进入生成的前端包，所以不能只看生产依赖审计。

新下载工具在需要代理的本机首次打包时出现 fetch failed。修正打包脚本后，@electron/get 5 在存在 HTTP(S)_PROXY 时启用其标准代理支持，保留 NO_PROXY；只作用于打包子进程，不修改系统设置。此后实际打包通过。构建环境需 Node >=22.12，本仓库约定使用 Node 22.18+。

### 可移除与应保留

- 已移除 `lodash-es`、`nanoid` 的直接依赖声明：src、scripts、tests 没有引用。PostCSS 等工具的间接 nanoid 不强行删除。
- 旧的 roarr/sprintf-js 随下载链替换退出锁文件；不替换成自制空壳，也不屏蔽告警。
- Vue SSR 由 Vue 和测试工具管理，升级到兼容版本，不手工删 node_modules。
- dmg-builder、Squirrel 虽然不是本项目的发布目标，但属于 electron-builder 的依赖结构；不在安装后手动裁剪。它们不属于 Fuze 的运行时功能。
- PaddleOCR、ONNX Runtime、Sharp、Electron 的 ffmpeg.dll 和 CPU 回退保留：分别承担识别、推理、图像处理、媒体截图与兼容性。不能为了减小体积删除所需运行库。

## 性能改动与证据

1. **传输前去重**：1024 × 576 RGBA 单帧为 2,359,296 字节。旧路径先跨进程复制，再在主进程 SHA-1；新路径在 capture worker 逐像素比较，只传变化帧。检查尺寸变化与单像素变化，避免采样哈希漏检。代价是截图进程额外保留一帧约 2.25 MiB。
2. **预热合并**：原来每次游戏阶段同步、临近等级循环都可触发预热、数据库检查和日志。现在并发请求合并，相同 GPU 设置下 30 秒内不重复后台预热；修改 GPU 模式可触发新预热，真正识别仍可按需初始化。
3. **闲置释放**：OCR 两分钟没有排队任务时释放 utility process，同时释放该进程持有的模型资源。新任务重置闲置计时；GPU 失败标记保留，避免重启后反复尝试已失败的 GPU。

### 固定像素微基准

运行 `node tests/electron/test-capture-worker-benchmark.mjs`，每个场景 120 帧；使用内存中的固定 RGBA 和 structuredClone 模拟数据复制，不访问游戏或保存桌面内容。时间为本机单次观察，受缓存、JIT 与 GC 影响；不作为整机 CPU 或游戏 FPS 结论。

| 场景 | 旧路径待传像素 | 新路径待传像素 | 减少 | 旧/新耗时 |
| --- | ---: | ---: | ---: | ---: |
| 静态画面 | 270 MiB | 2.25 MiB | 99.2% | 352.33 / 121.43 ms |
| 每十帧变化一次 | 270 MiB | 27 MiB | 90% | 309.76 / 132.92 ms |
| 每帧变化 | 270 MiB | 270 MiB | 0% | 302.00 / 187.78 ms |

变化帧计数均一致：1、12、120。真实游戏中的背景通常持续变化，所以不能把静态场景的 99.2% 外推到整个对局；等级门禁和标题缓存仍是主要节流手段。

## 验证与后续

- 66 个单元测试文件、294 项测试通过；覆盖去重、单像素变化、尺寸变化、会话重置、预热合并、闲置释放与重建。
- lint、TypeScript、前端与 Electron 构建通过；独立目录 npm 10 锁文件安装通过，审计 0 项。
- 固定 OCR 样本验证覆盖三卡、局部缺失、无匹配与多种输入尺寸/格式。
- 真实打包预检通过：四个窗口就绪，GPU/CPU 固定样本、GPU 子进程退出后的 CPU 恢复、持续截图均通过；合计 22.948 秒。最终版本安装包仍须执行同样检查，报告留在本地 build 的验证目录。
- GitHub Actions 的结果应独立查看；本地检查通过不等于云端运行通过。
- 尚未取得本版真实国服整局 CPU/GPU/FPS 对照，不能承诺实际降幅。后续重点测普通画面、三卡出现、局部重选、领取后消失、赛后资源释放。
- 进一步的 GPU 图像预处理或按标题区域传输涉及坐标、门禁和多显示器兼容性，本版不未经对局验证就替换整条链路。
