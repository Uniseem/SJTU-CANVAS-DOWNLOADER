; Inno Setup script for the Windows installer. Built by apps/windows/build.ps1
; -Installer, which passes the published app folder, version and architecture:
;   ISCC /DSourceDir=<dist\SJTUCanvasDownloader> /DAppVersion=1.0.0 /DArch=x64 SJTUCanvasDownloader.iss
;
; A per-user installation: no administrator rights and no UAC prompt. The app
; goes to %LOCALAPPDATA%\Programs\SJTU Canvas Downloader; the download list,
; settings and saved login stay in %LOCALAPPDATA%\SJTU Canvas Downloader and
; survive updates and uninstallation.

#ifndef SourceDir
  #define SourceDir "..\dist\SJTUCanvasDownloader"
#endif
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#ifndef Arch
  #define Arch "x64"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

#define AppName "SJTU Canvas Downloader"
#define AppExe "SJTUCanvasDownloader.exe"
#define AppUrl "https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER"

[Setup]
AppId={{CC6EEAB1-6437-420C-BE01-CBA78220B01E}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=SJTU Canvas Downloader contributors
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppCopyright=Copyright (c) 2026 SJTU Canvas Downloader contributors
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
UsePreviousAppDir=yes
MinVersion=10.0.19041
#if Arch == "arm64"
ArchitecturesAllowed=arm64
ArchitecturesInstallIn64BitMode=arm64
#else
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
WizardStyle=modern dynamic
SetupIconFile=..\SJTUCanvasDownloader\Assets\AppIcon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir={#OutputDir}
OutputBaseFilename=SJTUCanvasDownloader-win-{#Arch}-setup
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes
; Offer to close a running copy (its downloads resume on the next launch).
CloseApplications=yes
RestartApplications=no
ShowLanguageDialog=no
#ifdef Sign
SignTool=sjtucanvas
SignedUninstaller=yes
#endif

[Languages]
Name: "chinesesimplified"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
