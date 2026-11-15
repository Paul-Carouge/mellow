; Mellow's Windows installer (ADR-0060), compiled by Inno Setup 6.7 from the folder that
; `cargo xtask windows-package` lays out: mellow.exe, LICENSE.txt, mellow.ico, the wizard pictures
; and generated.iss (version, colours and picture lists from design/tokens.json).
;
; One screen: the welcome page with its picture and an "Install" button, then the progress bar,
; then Mellow opens and the installer closes. Per user, without administrator rights: no UAC
; prompt, no folder, component or shortcut choice, and an update or a reinstall asks nothing.
;
; Command line, beyond Inno Setup's own (/VERYSILENT, /SUPPRESSMSGBOXES, /NORESTART, /LOG=...):
;   /UPDATE  the running Mellow started this installer and is quitting: wait for it to exit
;            (up to UpdateGraceSeconds) before stopping it.
;   /LAUNCH  open Mellow at the end even when silent ("Restart to update").

#include "generated.iss"

; Seconds the app that started an update gets to finish quitting (leaving a call, signing out
; when asked) before the installer stops it.
#define UpdateGraceSeconds 60

[Setup]
; Never change: Windows finds the installed copy, its uninstaller and its settings by this id.
AppId={{CAD4F275-B047-40C0-A309-245EBA2DE460}
AppName=Mellow
AppVersion={#MellowVersion}
AppVerName=Mellow {#MellowVersion}
AppPublisher=Mellow
AppPublisherURL=https://github.com/Paul-Carouge/mellow
AppSupportURL=https://github.com/Paul-Carouge/mellow/issues
AppUpdatesURL=https://github.com/Paul-Carouge/mellow/releases
AppCopyright=AGPL-3.0-or-later
VersionInfoVersion={#MellowVersion}
VersionInfoProductName=Mellow
VersionInfoDescription=Mellow installer
; Per user, never elevated: %LOCALAPPDATA%\Programs\Mellow, HKCU, the user's Start menu.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Mellow
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
DisableWelcomePage=no
DisableFinishedPage=yes
ShowLanguageDialog=no
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; Windows 10 1809: GPUI's DirectComposition and Windows.Graphics.Capture's window exclusion.
MinVersion=10.0.17763
; The [Code] below stops a running Mellow by itself; Restart Manager would ask first.
CloseApplications=no
RestartApplications=no
SetupIconFile=mellow.ico
UninstallDisplayIcon={app}\mellow.exe
UninstallDisplayName=Mellow
WizardStyle=modern dynamic hidebevels
WizardBackColor={#BackColorLight}
WizardBackColorDynamicDark={#BackColorDark}
WizardImageFile={#WizardImages}
WizardSmallImageFile={#WizardSmallImages}
WizardImageStretch=yes
Compression=lzma2/ultra64
SolidCompression=yes
OutputDir=output
OutputBaseFilename={#OutputBaseName}
SetupLogging=yes
; Inno Setup's RedirectionGuard shields an elevated installer from planted junctions. This one
; never runs elevated, and the mitigation would pass to the Mellow it opens, breaking the user's
; own junctions and links there (ADR-0060).
RedirectionGuard=no
; Code signing comes later (ADR-0060): define a sign tool here (SignTool=mellow $f) and pass
; /Smellow=... to ISCC; it then signs this installer and its uninstaller. mellow.exe must be
; signed before it is packed.

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"

[Messages]
en.SetupWindowTitle=Install %1
fr.SetupWindowTitle=Installer %1
en.WelcomeLabel1=Install Mellow
fr.WelcomeLabel1=Installer Mellow
en.WelcomeLabel2=Your servers, conversations and calls, on the instance you choose.%n%nMellow installs for you only, in a few seconds and without administrator rights, then opens by itself.
fr.WelcomeLabel2=Tes serveurs, tes conversations et tes appels, sur l’instance de ton choix.%n%nMellow s’installe pour toi seulement, en quelques secondes et sans droits d’administrateur, puis s’ouvre tout seul.

[Files]
Source: "mellow.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; The identifier GPUI gives the process, so the taskbar and notifications group under this entry.
Name: "{autoprograms}\Mellow"; Filename: "{app}\mellow.exe"; AppUserModelID: "org.mellow.desktop"

[Registry]
; Written by the app for its notifications; only removed here.
Root: HKCU; Subkey: "Software\Classes\AppUserModelId\org.mellow.desktop"; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\mellow.exe"; Flags: nowait; Check: ShouldOpenMellow

[InstallDelete]
; The former YouTube player window (ADR-0061, superseded by ADR-0063), from an older version.
Type: files; Name: "{app}\mellow-player.exe"

[UninstallDelete]
; Updates being staged; preferences, drafts, the picture cache and the session stay, as on Linux.
Type: filesandordirs; Name: "{localappdata}\mellow\updates"
; The former player window's WebView2 folder (ADR-0061, superseded by ADR-0063), left by an older
; version: the web engine's own files, nothing of the user's.
Type: filesandordirs; Name: "{localappdata}\mellow\player-web-view"

[Code]
function HasSwitch(const Name: String): Boolean;
var
  Index: Integer;
begin
  Result := False;
  for Index := 1 to ParamCount do
    if CompareText(ParamStr(Index), Name) = 0 then
      Result := True;
end;

{ Kernel32's BOOL is four bytes: read as an Integer, not a one-byte Boolean. }
function SetEnvironmentVariable(Name, Value: String): Integer;
  external 'SetEnvironmentVariableW@kernel32.dll stdcall';

{ Waits up to GraceSeconds for the Mellow installed here to exit, then stops it. Only this
  copy, found by its path: another account's Mellow or a development build keeps running. The
  path reaches PowerShell through the environment, never inside the script, so no character of
  it (a quote in a user name) can change the command. }
procedure StopInstalledMellow(GraceSeconds: Integer);
var
  Executable, Script: String;
  ExitCode: Integer;
begin
  Executable := ExpandConstant('{app}\mellow.exe');
  if not FileExists(Executable) then
    Exit;
  if SetEnvironmentVariable('MELLOW_EXE', Executable) = 0 then
  begin
    Log('Could not pass the installed path to PowerShell');
    Exit;
  end;
  Script := '$running = @(Get-Process -Name mellow -ErrorAction SilentlyContinue | ' +
    'Where-Object { $_.Path -eq $env:MELLOW_EXE });';
  if GraceSeconds > 0 then
    Script := Script + ' $running | Wait-Process -Timeout ' + IntToStr(GraceSeconds) +
      ' -ErrorAction SilentlyContinue;';
  Script := Script + ' $running | Stop-Process -Force -ErrorAction SilentlyContinue;' +
    ' $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue';
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "' + Script + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Log('Could not run PowerShell to stop a running Mellow');
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  { The welcome page is the last one before installing: its button installs. }
  if CurPageID = wpWelcome then
    WizardForm.NextButton.Caption := SetupMessage(msgButtonInstall);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  if HasSwitch('/UPDATE') then
    StopInstalledMellow({#UpdateGraceSeconds})
  else
    StopInstalledMellow(0);
  Result := '';
end;

function ShouldOpenMellow: Boolean;
begin
  Result := (not WizardSilent) or HasSwitch('/LAUNCH');
end;

{ After the uninstaller's confirmation, before any file goes. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    StopInstalledMellow(0);
end;
