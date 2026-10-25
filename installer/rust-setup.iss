; Extract the selected payload; do not run installers or FUZE.
#ifndef AppRelease
  #define AppRelease "0.7.5"
#endif
#ifndef Payload
  #error Payload is required
#endif
#ifndef Output
  #error Output is required
#endif
[Setup]
AppId=FUZE.SimpleInstall
AppName=符择 FUZE
AppVersion={#AppRelease}
AppPublisher=FUZE Project
AppPublisherURL=https://github.com/ye602a/fuze-assistant
DefaultDirName={localappdata}\Programs\FUZE
DisableProgramGroupPage=yes
OutputDir={#Output}
OutputBaseFilename=FUZE-Setup-{#AppRelease}-windows-x64
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
CloseApplications=yes
CloseApplicationsFilter=*.exe,*.dll
RestartApplications=no
Compression=lzma2/normal
SolidCompression=no
WizardStyle=modern
SignedUninstaller=no
UninstallDisplayIcon={app}\fuze-ui-lab.exe

[Languages]
Name: "chinesesimplified"; MessagesFile: "languages\ChineseSimplified.isl"

[Messages]
FinishedLabel=FUZE v{#AppRelease} 已解压到所选目录，所需 VC++ DLL 已随程序提供。请通过 FUZE 快捷方式启动。%n%n安装完成后不会自动启动程序。

[Files]
Source: "{#Payload}\*"; DestDir: "{app}"; Excludes: "*.cmd,*.bat,*.ps1"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\FUZE"; Filename: "{app}\fuze-ui-lab.exe"; WorkingDir: "{app}"; IconFilename: "{app}\fuze-ui-lab.exe"
Name: "{userdesktop}\FUZE"; Filename: "{app}\fuze-ui-lab.exe"; WorkingDir: "{app}"; IconFilename: "{app}\fuze-ui-lab.exe"
