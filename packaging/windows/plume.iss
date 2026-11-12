; Windows installer for Plume (Inno Setup 6).
; Built by the release workflow:
;   ISCC /DAppVersion=1.2.3 /DSourceExe=path\to\plume.exe /DOutputDir=dist packaging\windows\plume.iss
;
; Installs per user (no admin prompt) into %LOCALAPPDATA%\Programs\Plume, so the
; built-in updater can replace Plume.exe in place without elevation.
; Requests, environments and settings live in %USERPROFILE%\.plume and are kept on uninstall.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\plume.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{6F1E2B7A-3C4D-4E8F-9A61-5B2C7D0E4F13}
AppName=Plume
AppVersion={#AppVersion}
AppVerName=Plume {#AppVersion}
AppPublisher=Rawz06 Studio
AppPublisherURL=https://github.com/Rawz06-Studio/Plume
AppSupportURL=https://github.com/Rawz06-Studio/Plume/issues
AppUpdatesURL=https://github.com/Rawz06-Studio/Plume/releases
DefaultDirName={autopf}\Plume
DefaultGroupName=Plume
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\Plume.exe
UninstallDisplayName=Plume
OutputDir={#OutputDir}
OutputBaseFilename=Plume-Setup-x86_64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
ShowLanguageDialog=auto

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Plume.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Plume"; Filename: "{app}\Plume.exe"
Name: "{autodesktop}\Plume"; Filename: "{app}\Plume.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Plume.exe"; Description: "{cm:LaunchProgram,Plume}"; Flags: nowait postinstall skipifsilent
