# v0.7.0 安装包封装

只封装现有正式发行 `FUZE-v0.7.0-windows-x64.zip`，程序构建仍为 `dc0957fd7617`。
原 ZIP SHA-256：`35ab61daad95f128310ab77b111ea9db4284264f8e930bb8d120faa2c110e892`。
不编译 Rust，不修改程序文件，不增加应用版本，不改变更新频道。

`FUZE-Setup-0.7.0-windows-x64.exe` 是未签名的自解压安装向导。
仅将原包文件解压到用户选择的目录（默认 `%LOCALAPPDATA%\Programs\FUZE`），
创建桌面和开始菜单快捷方式，直接以 `--desktop preview.json` 启动原版 Slint 程序。
不运行 `Install-local.cmd`、PowerShell、依赖安装或应用。卸载由 Inno 删除它安装的文件、
快捷方式和卸载登记，不递归清理用户配置、日志或其他旧安装。

原 ZIP 已含 ONNX Runtime、DirectML、dxcompiler 和 dxil，但没有 VC++ 运行库。
因此此封装不是全部依赖自带的离线独立运行包；缺少兼容 VC++ x64 运行库的机器仍可能
无法运行。原程序及更新器的依赖要求保持原样，此封装不会创建旧安装器的受管版本指针。

手动触发 `package-rust070-setup.yml` 下载并核对原 ZIP，使用 Inno 封装，
仅替换现有 `rust-v0.7.0` Release 的 Setup EXE、SHA-256 和封装回执，保留原 ZIP。
不运行安装器或 FUZE，不执行 Rust 构建及扩展验收。
这次交付证明封装来源和编译成功，不声称新增了实机安装验收。
