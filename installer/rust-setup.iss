; Inno is only the bootstrap UI/container. Native FUZE owns files, shortcuts and uninstall.
#ifndef Payload
  #error Payload is required
#endif
#ifndef Output
  #error Output is required
#endif
#ifndef Version
  #error Version is required
#endif
[Setup]
AppId=FUZE.Native.Bootstrap
AppName=符择 FUZE
AppVersion={#Version}
AppPublisher=FUZE Project
AppPublisherURL=https://github.com/ye602a/fuze-assistant
OutputDir={#Output}
OutputBaseFilename=FUZE-Setup-{#Version}-windows-x64
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CreateAppDir=no
Uninstallable=no
DisableWelcomePage=no
DisableDirPage=yes
DisableProgramGroupPage=yes
CloseApplications=no
RestartApplications=no
AllowCancelDuringInstall=no
Compression=lzma2/normal
SolidCompression=no
WizardStyle=modern
SetupLogging=no
SignedUninstaller=no
DisableReadyPage=no

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Messages]
WelcomeLabel2=此向导将安装符择 FUZE。当前安装包未签名。%n%n程序安装到当前用户目录，已有设置和个人记录保留。请先自行退出正在运行的 FUZE。%n%n安装完成后不会自动启动程序，也不会自动重启电脑。
FinishedLabel=FUZE 已安装。请从开始菜单的 Fuze Rust Preview 入口启动。%n%n该兼容名称暂时保留。卸载可通过 Windows 已安装应用或开始菜单进行，用户数据会保留。

[Files]
Source: "..\scripts\rust-setup-bootstrap.ps1"; DestDir: "{tmp}"; Flags: dontcopy
Source: "{#Payload}\*"; DestDir: "{tmp}\payload"; Flags: dontcopy recursesubdirs

[Code]
var
  Installed: Boolean;

function UpdateReadyMemo(Space, NewLine, MemoUserInfoInfo, MemoDirInfo, MemoTypeInfo, MemoComponentsInfo, MemoGroupInfo, MemoTasksInfo: String): String;
begin
  Result := '安装 FUZE {#Version}（未签名）' + NewLine + NewLine +
    '先核对本构建的 VC++ x64 运行库，再安装程序并创建开始菜单和卸载入口。' + NewLine +
    '缺少运行库时，下载及安装均需要你明确确认。取消不会继续安装 FUZE。' + NewLine +
    '保留设置、个人记录与缓存；不自动启动、不自动重启。';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Args: String;
begin
  Result := '';
  if Installed then exit;
  ExtractTemporaryFiles('{tmp}\*');
  Args := '-NoProfile -NonInteractive -File "' + ExpandConstant('{tmp}\rust-setup-bootstrap.ps1') +
    '" -Bundle "' + ExpandConstant('{tmp}\payload') + '"';
  if WizardSilent then Args := Args + ' -Unattended';
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    Args, ExpandConstant('{tmp}'), SW_HIDE, ewWaitUntilTerminated, Code) then
    Result := '无法启动 Windows PowerShell。请检查系统策略后重试；未完成安装。'
  else
    case Code of
      0: Installed := True;
      2: Result := '运行库安装已取消。FUZE 尚未安装；可退出或准备好运行库后重试。';
      3: Result := '缺少本构建要求的 VC++ x64 运行库。请交互运行安装器以查看微软下载引导。';
      4: Result := '运行库要求重新启动 Windows。请自行安排重启，之后重新运行本安装器；FUZE 尚未安装。';
    else
      Result := '安装未完成。请退出正在运行的 FUZE，并确认原始安装包及系统策略后重试。已有用户数据会保留。';
    end;
end;
