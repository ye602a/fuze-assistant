# v0.7.0 安装包封装

只封装现有正式发行 `FUZE-v0.7.0-windows-x64.zip`，程序构建仍为 `dc0957fd7617`。
原 ZIP SHA-256：`35ab61daad95f128310ab77b111ea9db4284264f8e930bb8d120faa2c110e892`。
主界面、OCR 和宿主沿用原文件；仅重编译更新器以支持本地 VC++ DLL。
应用版本保持 0.7.0，保留主程序 revision，另记 updaterRevision，不改变更新频道。

`FUZE-Setup-0.7.0-windows-x64.exe` 是未签名的自解压安装向导。
仅将原包文件解压到用户选择的目录（默认 `%LOCALAPPDATA%\Programs\FUZE`），
创建桌面和开始菜单快捷方式，使用 `fuze-update.exe --launch` 保持更新后的启动与回滚。
不运行 `Install-local.cmd`、PowerShell、依赖安装或应用。卸载由 Inno 删除它安装的文件、
快捷方式和卸载登记，不递归清理用户配置、日志或其他旧安装。

VC++ DLL 从策略固定的微软 VC_redist.x64.exe 解出，不执行运行库安装器。
按原生程序和 DLL 的普通/延迟导入递归选择所需 x64 CRT，随 EXE 放置。
更新器和便携入口优先检查本地 DLL；更新或回滚到缺少 CRT 的旧包时只补缺失 DLL，
不覆盖目标版本自带 DLL。运行库文件列入受管文件清单，随版本复制及清理。
Windows 10/11 自带的 UCRT 和系统组件不重复打包。

手动触发 `package-rust070-setup.yml` 下载并核对原 ZIP，使用 Inno 封装，
仅替换现有 `rust-v0.7.0` Release 的 Setup EXE、SHA-256 和封装回执，保留原 ZIP。
不运行安装器或 FUZE，只运行运行库选择/旧包补齐的定向测试并编译更新器。
这次交付证明封装来源和编译成功，不声称新增了实机安装验收。
