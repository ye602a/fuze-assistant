; Packaging only: the published v0.7.0 payload owns dependency checks, install and uninstall.
#ifndef Payload
  #error Payload is required
#endif
#ifndef Output
  #error Output is required
#endif
[Setup]
AppId=FUZE.Native.Bootstrap
AppName=符择 FUZE
AppVersion=0.7.0
AppPublisher=FUZE Project
AppPublisherURL=https://github.com/ye602a/fuze-assistant
OutputDir={#Output}
OutputBaseFilename=FUZE-Setup-0.7.0-windows-x64
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CreateAppDir=no
Uninstallable=no
DisableWelcomePage=no
DisableDirPage=yes
DisableProgramGroupPage=yes
CloseApplications=no
RestartApplications=no
Compression=lzma2/normal
SolidCompression=no
WizardStyle=modern
SignedUninstaller=no

[Languages]
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"

[Messages]
WelcomeLabel2=此安装包封装已发布的 FUZE v0.7.0，程序内容保持原样，安装包未签名。%n%n下一步将打开随包安装窗口，按原有提示完成运行库检查与安装；窗口提示 Press Enter 时，请按回车继续。%n%n安装后不会自动启动 FUZE，不自动重启 Windows。
FinishedLabel=FUZE v0.7.0 安装完成。请从开始菜单的 Fuze Rust Preview 入口启动。%n%n该名称为已有安装保留。卸载通过 Windows 已安装应用进行，沿用原版的数据保留规则。

[Files]
Source: "{#Payload}\*"; DestDir: "{tmp}\payload"; Flags: dontcopy recursesubdirs

[Code]
var
  Installed: Boolean;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Args: String;
begin
  Result := '';
  if Installed then exit;
  ExtractTemporaryFiles('{tmp}\payload\*');
  { Run the original entry point unchanged, including its original confirmations. }
  Args := '/d /c ""' + ExpandConstant('{tmp}\payload\Install-local.cmd') + '""';
  if not Exec(ExpandConstant('{cmd}'), Args, ExpandConstant('{tmp}\payload'),
    SW_SHOWNORMAL, ewWaitUntilTerminated, Code) then
  begin
    Result := '无法打开随包安装窗口。安装未完成。';
    exit;
  end;
  if Code <> 0 then
  begin
    Result := '随包安装未完成。请查看安装窗口提示，准备好依赖后重试；已有用户数据保留。';
    exit;
  end;
  { The original installer returns success on user cancellation; verify its existing pointer. }
  Args := '-NoProfile -NonInteractive -Command "$p=Join-Path $env:LOCALAPPDATA ''FuzeRustPreview\releases\current.json''; try { if ((Get-Content -LiteralPath $p -Raw -ErrorAction Stop | ConvertFrom-Json).current.revision -eq ''dc0957fd7617'') { exit 0 } } catch {}; exit 1"';
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Args, ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, Code) then
    Result := '无法确认安装结果。请查看 Windows 已安装应用。'
  else if Code <> 0 then
    Result := '安装已取消或尚未完成，没有确认 v0.7.0 安装记录。'
  else
    Installed := True;
end;
