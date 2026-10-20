; Extract the original v0.7.0 files; do not run installers or FUZE.
#ifndef Payload
  #error Payload is required
#endif
#ifndef Output
  #error Output is required
#endif
[Setup]
AppId=FUZE.SimpleInstall
AppName=符择 FUZE
AppVersion=0.7.0
AppPublisher=FUZE Project
AppPublisherURL=https://github.com/ye602a/fuze-assistant
DefaultDirName={localappdata}\Programs\FUZE
DisableProgramGroupPage=yes
OutputDir={#Output}
OutputBaseFilename=FUZE-Setup-0.7.0-windows-x64
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CloseApplications=no
RestartApplications=no
Compression=lzma2/normal
SolidCompression=no
WizardStyle=modern
SignedUninstaller=no
UninstallDisplayIcon={app}\fuze-ui-lab.exe

[Languages]
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"

[Messages]
FinishedLabel=FUZE v0.7.0 已解压到所选目录。请通过 FUZE 快捷方式启动。%n%n安装包未签名，未安装 VC++ 运行库。安装完成后不会自动启动程序。

[Files]
Source: "{#Payload}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\FUZE"; Filename: "{app}\fuze-ui-lab.exe"; Parameters: "--desktop ""{app}\preview.json"""; WorkingDir: "{app}"
Name: "{userdesktop}\FUZE"; Filename: "{app}\fuze-ui-lab.exe"; Parameters: "--desktop ""{app}\preview.json"""; WorkingDir: "{app}"