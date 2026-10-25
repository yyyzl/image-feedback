#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef StageDir
  #error StageDir must point to the complete, audited distribution tree
#endif
#ifndef OutputDir
  #error OutputDir must be supplied
#endif

[Setup]
AppId={{A57E2C63-8619-4B9D-BC75-9B7DE154A621}
AppName=Image Feedback
AppVersion={#AppVersion}
AppPublisher=Image Feedback contributors
AppPublisherURL=https://github.com/yyyzl/image-feedback
AppSupportURL=https://github.com/yyyzl/image-feedback/issues
DefaultDirName={localappdata}\Programs\ImageFeedback
DefaultGroupName=Image Feedback
PrivilegesRequired=lowest
ArchitecturesAllowed=x64os
ArchitecturesInstallIn64BitMode=x64os
MinVersion=10.0
UsePreviousAppDir=yes
DisableProgramGroupPage=yes
AppMutex=Local\ImageFeedback.Running
CloseApplications=no
RestartApplications=no
UninstallDisplayIcon={app}\ImageFeedback.exe
SetupIconFile=..\..\assets\app-icon.ico
LicenseFile=..\..\LICENSE
OutputDir={#OutputDir}
OutputBaseFilename=ImageFeedback-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#StageDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Image Feedback"; Filename: "{app}\ImageFeedback.exe"; WorkingDir: "{app}"
Name: "{group}\Image Feedback (Software rendering)"; Filename: "{app}\ImageFeedback.exe"; Parameters: "--software-rendering"; WorkingDir: "{app}"
Name: "{userdesktop}\Image Feedback"; Filename: "{app}\ImageFeedback.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\ImageFeedback.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\ImageFeedback.exe"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\.ifp\OpenWithProgids"; ValueType: string; ValueName: "ImageFeedback.Project"; ValueData: ""; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\ImageFeedback.Project"; ValueType: string; ValueName: ""; ValueData: "Image Feedback project"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\ImageFeedback.Project\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\ImageFeedback.exe,0"
Root: HKCU; Subkey: "Software\Classes\ImageFeedback.Project\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\ImageFeedback.exe"" --open ""%1"""

[Run]
Filename: "{app}\ImageFeedback.exe"; Description: "Launch Image Feedback"; Flags: nowait postinstall skipifsilent

[Code]
function Running(): Boolean;
begin
  Result := CheckForMutexes('Local\ImageFeedback.Running');
end;

function InitializeSetup(): Boolean;
begin
  Result := not Running();
  if (not Result) and (not WizardSilent()) then
    MsgBox('Please save your work and close BOTH the editor and tray entry before updating Image Feedback. No process will be forcibly closed.', mbError, MB_OK);
end;

function InitializeUninstall(): Boolean;
begin
  Result := not Running();
  if (not Result) and (not UninstallSilent()) then
    MsgBox('Please save your work and close BOTH the editor and tray entry first. Your images and project data will be kept.', mbError, MB_OK);
end;
