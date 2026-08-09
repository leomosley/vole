#ifndef AppVersion
  #define AppVersion "0.1.0-dev"
#endif

[Setup]
AppId={{D21B15F7-88A9-4F19-9EA8-E0BA257672C8}
AppName=VOLE
AppVersion={#AppVersion}
AppPublisher=VOLE
AppPublisherURL=https://github.com/leomosley/vole
AppSupportURL=https://github.com/leomosley/vole/issues
DefaultDirName={autopf}\VOLE
DefaultGroupName=VOLE
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=VOLE-Setup-{#AppVersion}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\vole.exe
CloseApplications=yes
CloseApplicationsFilter=vole.exe
RestartApplications=no
UsePreviousAppDir=yes
UsePreviousGroup=yes
UsePreviousTasks=yes

[Tasks]
Name: "startup"; Description: "Launch VOLE when I sign in"; GroupDescription: "Startup:"; Flags: checkedonce

[Files]
Source: "..\target\release\vole.exe"; DestDir: "{app}"; Flags: ignoreversion restartreplace

[Icons]
Name: "{group}\VOLE"; Filename: "{app}\vole.exe"
Name: "{group}\Uninstall VOLE"; Filename: "{uninstallexe}"

; Remove the legacy Run-key autostart left by older installs. Autostart is now a
; scheduled task, and an elevated exe launched from a Run key fails to start
; (CreateProcess cannot elevate), so the stale value must go.
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "VOLE"; ValueType: none; Flags: deletevalue uninsdeletevalue

; VOLE requires administrator rights, so autostart runs through a logon
; scheduled task registered with the highest privileges instead of a Run key.
; A Run key would raise a UAC prompt at every logon. The elevated app manages
; the task through its own CLI, keeping the schtasks details in one place.
; The Finish-page launch uses shellexec because Inno runs postinstall items
; de-elevated; a plain CreateProcess of the elevated exe would fail with 740.
[Run]
Filename: "{app}\vole.exe"; Parameters: "--enable-autostart"; Tasks: startup; Flags: runhidden waituntilterminated
Filename: "{app}\vole.exe"; Description: "Launch VOLE"; Flags: nowait postinstall skipifsilent shellexec

[UninstallRun]
Filename: "{app}\vole.exe"; Parameters: "--disable-autostart"; Flags: runhidden waituntilterminated; RunOnceId: "DisableAutostart"

[Code]
procedure StopRunningVole;
var
  Attempt: Integer;
  ResultCode: Integer;
begin
  for Attempt := 1 to 3 do
  begin
    Log(Format('Stopping running VOLE process (attempt %d)', [Attempt]));
    Exec(
      ExpandConstant('{sys}\taskkill.exe'),
      '/F /T /IM vole.exe',
      '',
      SW_HIDE,
      ewWaitUntilTerminated,
      ResultCode
    );
    Sleep(500);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopRunningVole;
  Result := '';
end;
