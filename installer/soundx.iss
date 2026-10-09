#ifndef AppVersion
  #define AppVersion "0.3.0"
#endif
#define Root AddBackslash(SourcePath) + "..\"

[Setup]
#ifdef IntegrationTestRoot
AppId=soundx.integration-test
AppName=soundx integration test
DefaultDirName={#IntegrationTestRoot}\app
DefaultGroupName=soundx integration test
#else
AppId={{D371F55B-EDBB-4CC3-8A15-0D8339467947}
AppName=soundx
DefaultDirName={autopf}\soundx
DefaultGroupName=soundx
#endif
AppVersion={#AppVersion}
AppPublisher=soundx
OutputDir={#Root}dist
OutputBaseFilename=soundx-{#AppVersion}-setup
SetupIconFile={#Root}assets\soundx.ico
UninstallDisplayIcon={app}\soundx.exe
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ChangesEnvironment=yes
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

[Tasks]
Name: "agent_codex"; Description: "Install soundx Skill and MCP for Codex (current user)"; GroupDescription: "Agent integrations:"; Flags: unchecked
Name: "agent_claude"; Description: "Install soundx Skill and MCP for Claude Code (current user)"; GroupDescription: "Agent integrations:"; Flags: unchecked
Name: "agent_opencode"; Description: "Install soundx Skill and MCP for OpenCode (current user)"; GroupDescription: "Agent integrations:"; Flags: unchecked
Name: "agent_agy"; Description: "Install soundx Skill and MCP for AGY CLI / Antigravity (current user)"; GroupDescription: "Agent integrations:"; Flags: unchecked

[Files]
Source: "{#Root}target\release\soundx.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Root}THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
#if FileExists(Root + "LICENSE-MIT")
Source: "{#Root}LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
#endif
#if FileExists(Root + "LICENSE-LGPL")
Source: "{#Root}LICENSE-LGPL"; DestDir: "{app}"; Flags: ignoreversion
#endif
Source: "{#Root}docs\*.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "{#Root}pages\en\README.md"; DestDir: "{app}\pages\en"; Flags: ignoreversion
Source: "{#Root}pages\zh-TW\README.md"; DestDir: "{app}\pages\zh-TW"; Flags: ignoreversion
Source: "{#Root}skills\soundx\SKILL.md"; DestDir: "{app}\skills\soundx"; Flags: ignoreversion

[Icons]
Name: "{group}\soundx Command Prompt"; Filename: "{cmd}"; Parameters: "/K soundx --help"; WorkingDir: "{app}"; IconFilename: "{app}\soundx.exe"

[Code]
const
  MachineEnvironment = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  UserEnvironment = 'Environment';

function EnvironmentRoot(): Integer;
begin
  if IsAdminInstallMode then
    Result := HKLM
  else
    Result := HKCU;
end;

function EnvironmentKey(): String;
begin
  if IsAdminInstallMode then
    Result := MachineEnvironment
  else
    Result := UserEnvironment;
end;

function NormalizePath(Value: String): String;
begin
  Result := Lowercase(Trim(Value));
  if (Length(Result) >= 2) and (Result[1] = '"') and
     (Result[Length(Result)] = '"') then
    Result := Copy(Result, 2, Length(Result) - 2);
  while (Length(Result) > 3) and (Result[Length(Result)] = '\') do
    Delete(Result, Length(Result), 1);
end;

function UpdatePath(Add: Boolean): Boolean;
var
  Original, Segment, Updated, Target: String;
  Position, Separator: Integer;
  Found, FirstKept: Boolean;
begin
  Result := False;
  if not RegQueryStringValue(EnvironmentRoot(), EnvironmentKey(), 'Path', Original) then
    Original := '';
  Target := NormalizePath(ExpandConstant('{app}'));
  Updated := '';
  Found := False;
  FirstKept := True;
  Position := 1;
  while Position <= Length(Original) + 1 do begin
    Separator := Pos(';', Copy(Original, Position, MaxInt));
    if Separator = 0 then
      Separator := Length(Original) - Position + 2;
    Segment := Copy(Original, Position, Separator - 1);
    if NormalizePath(Segment) = Target then
      Found := True
    else begin
      if not FirstKept then Updated := Updated + ';';
      Updated := Updated + Segment;
      FirstKept := False;
    end;
    Position := Position + Separator;
  end;
  if Add then begin
    if Found then begin
      Result := True;
      Exit;
    end;
    Updated := Original;
    if (Updated <> '') and (Updated[Length(Updated)] <> ';') then
      Updated := Updated + ';';
    Updated := Updated + ExpandConstant('{app}');
  end else if not Found then
    Exit;
  Result := RegWriteExpandStringValue(EnvironmentRoot(), EnvironmentKey(), 'Path', Updated);
end;

function ProfileArguments(): String;
begin
#ifdef IntegrationTestRoot
  Result := ' --home "{#IntegrationTestRoot}\profile"';
#else
  Result := '';
#endif
end;

procedure InstallAgent(Agent: String);
var
  ExitCode: Integer;
begin
  if not WizardIsTaskSelected('agent_' + Agent) then Exit;
  if not Exec(ExpandConstant('{app}\soundx.exe'), 'integrate install --agent ' + Agent + ProfileArguments(),
    ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) or (ExitCode <> 0) then begin
    Log('soundx integration failed for ' + Agent);
    SuppressibleMsgBox('soundx was installed, but the ' + Agent + ' integration failed. Existing conflicting settings were preserved. Run soundx integrate install --agent ' + Agent + ' in a terminal for details.', mbError, MB_OK, IDOK);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then begin
#ifndef IntegrationTestRoot
    if not UpdatePath(True) then
      Log('Failed to add soundx installation directory to PATH');
#endif
    InstallAgent('codex');
    InstallAgent('claude');
    InstallAgent('opencode');
    InstallAgent('agy');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ExitCode: Integer;
begin
  if CurUninstallStep = usUninstall then begin
    if not Exec(ExpandConstant('{app}\soundx.exe'), 'integrate remove --all --only-executable "' + ExpandConstant('{app}\soundx.exe') + '"' + ProfileArguments(),
      ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ExitCode) or (ExitCode <> 0) then
      SuppressibleMsgBox('Some soundx agent integrations could not be removed. Existing or modified user settings were preserved. See the soundx integration manifest in your user profile for the preserved files.', mbInformation, MB_OK, IDOK);
#ifndef IntegrationTestRoot
    UpdatePath(False);
#endif
  end;
end;
