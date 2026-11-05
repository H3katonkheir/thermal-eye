; Thermal Eye installer. Built by build.ps1, which passes /DAppVersion=x.y.z.
;
; The app always goes in Program Files: the elevated scheduled task may only point
; at a file ordinary programs cannot replace, so the folder is not a choice.

#ifndef AppVersion
  #error Build with build.ps1, which sets AppVersion
#endif

[Setup]
AppId={{7E2B6C41-9A3D-4F58-8C1E-5D0F3A7B9E12}
AppName=Thermal Eye
AppVersion={#AppVersion}
AppVerName=Thermal Eye {#AppVersion}
AppPublisher=Dominique Bastenie
AppPublisherURL=https://github.com/H3katonkheir/thermal-eye
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\ThermalEye
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
CloseApplications=no
OutputDir=..\dist
OutputBaseFilename=ThermalEyeSetup-{#AppVersion}
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\thermal-eye.exe
UninstallDisplayName=Thermal Eye
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Tasks]
Name: autostart; Description: "Start with Windows"
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked
Name: pawnio; Description: "Install the PawnIO driver (needed for CPU, RAM and motherboard temperatures)"; \
    GroupDescription: "Hardware access (the FACEIT anti-cheat blocks this driver):"; Check: NeedsPawnIO and not IsUpdate

[Files]
Source: "..\dist\thermal-eye.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\thermal-eye-sensors.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\Thermal Eye"; Filename: "{app}\thermal-eye.exe"
Name: "{autodesktop}\Thermal Eye"; Filename: "{app}\thermal-eye.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\thermal-eye-sensors.exe"; Parameters: "--register-task"; Flags: runhidden waituntilterminated; \
    StatusMsg: "Setting up sensor access..."
; As the signed-in user: the startup entry belongs to them, not to the elevated installer.
Filename: "{app}\thermal-eye.exe"; Parameters: "--autostart on"; Tasks: autostart; Check: not IsUpdate; \
    Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\thermal-eye.exe"; Parameters: "--autostart off"; Tasks: not autostart; Check: not IsUpdate; \
    Flags: runasoriginaluser runhidden waituntilterminated
Filename: "{app}\thermal-eye.exe"; Description: "Open Thermal Eye"; \
    Flags: postinstall nowait skipifsilent runasoriginaluser
; After an in-app update, reopen what the update closed.
Filename: "{app}\thermal-eye.exe"; Flags: nowait runasoriginaluser; Check: IsUpdate

[UninstallRun]
Filename: "{app}\thermal-eye.exe"; Parameters: "--autostart off"; Flags: runhidden; RunOnceId: "RemoveStartupEntry"
Filename: "{app}\thermal-eye-sensors.exe"; Parameters: "--unregister-task"; Flags: runhidden; RunOnceId: "RemoveTask"

[UninstallDelete]
; Inno only removes folders it created, which misses a folder from an older setup.
Type: dirifempty; Name: "{app}"

[Code]
const
  // Pinned to one release and checked against its hash, because it runs elevated.
  PawnIOUrl = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe';
  PawnIOHash = '1f519a22e47187f70a1379a48ca604981c4fcf694f4e65b734aaa74a9fba3032';
  RebootRequired = 3010;

var
  RestartNeeded: Boolean;

// Set by the widget when it runs an update. An update must leave the user's
// choices alone, such as having turned off Start with Windows or skipped PawnIO.
function IsUpdate: Boolean;
begin
  Result := ExpandConstant('{param:update|0}') = '1';
end;

function NeedsPawnIO: Boolean;
begin
  Result := not RegKeyExists(HKLM, 'SYSTEM\CurrentControlSet\Services\PawnIO');
end;

procedure RunTaskKill(const Arguments: String);
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), Arguments, '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

// Asks the widget to close, so it saves its settings, then makes sure. The helper
// stops by itself when the widget goes.
procedure CloseRunningApp;
begin
  RunTaskKill('/im thermal-eye.exe');
  Sleep(1000);
  RunTaskKill('/f /im thermal-eye.exe');
  RunTaskKill('/f /im thermal-eye-sensors.exe');
  Sleep(500);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  CloseRunningApp;
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    CloseRunningApp;
end;

procedure WarnAboutPawnIO;
begin
  SuppressibleMsgBox(
    'The PawnIO driver could not be installed. Thermal Eye will still run, and will ' +
    'offer the download when you open it.', mbInformation, MB_OK, IDOK);
end;

procedure InstallPawnIO;
var
  Code: Integer;
begin
  WizardForm.StatusLabel.Caption := 'Installing the PawnIO driver...';
  try
    DownloadTemporaryFile(PawnIOUrl, 'PawnIO_setup.exe', PawnIOHash, nil);
  except
    WarnAboutPawnIO;
    Exit;
  end;

  if not Exec(ExpandConstant('{tmp}\PawnIO_setup.exe'), '-install -silent', '', SW_HIDE,
      ewWaitUntilTerminated, Code) then
    WarnAboutPawnIO
  else if Code = RebootRequired then
    RestartNeeded := True
  else if Code <> 0 then
    WarnAboutPawnIO;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('pawnio') then
    InstallPawnIO;
end;

function NeedRestart: Boolean;
begin
  Result := RestartNeeded;
end;
