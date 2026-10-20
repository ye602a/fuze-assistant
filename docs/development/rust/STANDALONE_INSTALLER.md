# v0.7.0 安装包封装

只封装现有正式发行 `FUZE-v0.7.0-windows-x64.zip`，程序构建仍为 `dc0957fd7617`。
原 ZIP SHA-256：`35ab61daad95f128310ab77b111ea9db4284264f8e930bb8d120faa2c110e892`。
不编译 Rust，不修改程序文件，不增加应用版本，不改变更新频道。

`FUZE-Setup-0.7.0-windows-x64.exe` 是未签名的自解压安装向导。
它打开原包的 `Install-local.cmd`，保留原有运行库检查、确认窗口、安装和卸载逻辑。
安装窗口出现 `Press Enter` 时需按回车完成。安装后不自动启动 FUZE，不自动重启。
原有 Preview 名称与提示也保持原样。

手动触发 `package-rust070-setup.yml` 下载并核对原 ZIP，使用 Inno 封装，
将 EXE、SHA-256 和封装回执补充到现有 `rust-v0.7.0` Release。
不运行安装器或 FUZE，不执行 Rust 构建及扩展验收。
这次交付证明封装来源和编译成功，不声称新增了实机安装验收。
