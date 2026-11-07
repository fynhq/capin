; Per-user Capin installer.
;
; CI supplies these defines:
;   AppVersion, VersionInfoVersion, Arch, ArchAllowed, ArchMode,
;   SourceDir, OutputDir, OutputBaseFilename, RepoRoot

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

#ifndef VersionInfoVersion
  #define VersionInfoVersion "0.0.0.0"
#endif

#ifndef Arch
  #define Arch "x64"
#endif

#ifndef ArchAllowed
  #define ArchAllowed "x64os"
#endif

#ifndef ArchMode
  #define ArchMode "x64os"
#endif

#ifndef SourceDir
  #define SourceDir "Capin"
#endif

#ifndef OutputDir
  #define OutputDir "."
#endif

#ifndef OutputBaseFilename
  #define OutputBaseFilename "Capin_Windows_" + Arch + "_setup"
#endif

#ifndef RepoRoot
  #define RepoRoot "."
#endif

[Setup]
AppId={{4F2A7C9D-6E31-4B58-9A0C-7D5E3B1F8A62}
AppName=Capin
; Inno's fallback for an omitted AppVerName is "AppName version AppVersion",
; which puts the version into the Apps & features name column. Keep both
; fields explicit so the name stays fixed; the version lives in DisplayVersion.
AppVerName=Capin
UninstallDisplayName=Capin
AppVersion={#AppVersion}
AppPublisher=Capin
AppPublisherURL=https://github.com/fynhq/capin
AppSupportURL=https://github.com/fynhq/capin
AppUpdatesURL=https://github.com/fynhq/capin/releases
AppComments=https://github.com/fynhq/capin
AppContact=https://github.com/fynhq/capin
AppCopyright=Copyright (c) 2026 Capin
DefaultDirName={localappdata}\Programs\Capin
DefaultGroupName=Capin
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseFilename}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchMode}
UninstallDisplayIcon={app}\capin.exe
LicenseFile={#SourceDir}\LICENSE
CloseApplications=yes
RestartApplications=no
SetupIconFile={#RepoRoot}\assets\icon\capin_icon.ico
VersionInfoVersion={#VersionInfoVersion}
VersionInfoDescription=Capin Setup
VersionInfoProductName=Capin

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\capin.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Capin"; Filename: "{app}\capin.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Capin"; Filename: "{app}\capin.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\capin.exe"; Description: "{cm:LaunchProgram,Capin}"; WorkingDir: "{app}"; Flags: nowait postinstall skipifsilent

[Code]
function NormalizeVersion(const V: String): String;
var
  P: Integer;
begin
  Result := Trim(V);
  if (Length(Result) > 0) and ((Result[1] = 'v') or (Result[1] = 'V')) then
    Result := Copy(Result, 2, Length(Result));
  P := Pos('-', Result);
  if P > 0 then
    Result := Copy(Result, 1, P - 1);
  P := Pos('+', Result);
  if P > 0 then
    Result := Copy(Result, 1, P - 1);
end;

function TakeVersionPart(var S: String): Integer;
var
  P: Integer;
begin
  P := Pos('.', S);
  if P > 0 then
  begin
    Result := StrToIntDef(Copy(S, 1, P - 1), 0);
    Delete(S, 1, P);
  end
  else
  begin
    Result := StrToIntDef(S, 0);
    S := '';
  end;
end;

function CompareVersions(const A, B: String): Integer;
var
  SA, SB: String;
  I, PA, PB: Integer;
begin
  SA := NormalizeVersion(A);
  SB := NormalizeVersion(B);
  Result := 0;
  for I := 1 to 4 do
  begin
    PA := TakeVersionPart(SA);
    PB := TakeVersionPart(SB);
    if PA < PB then
    begin
      Result := -1;
      Exit;
    end;
    if PA > PB then
    begin
      Result := 1;
      Exit;
    end;
  end;
end;

function GetInstalledVersion(var Version: String): Boolean;
var
  Key, InstallLocation, ExePath, FileVersion: String;
begin
  Result := False;
  Version := '';
  Key := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{4F2A7C9D-6E31-4B58-9A0C-7D5E3B1F8A62}_is1';
  if RegQueryStringValue(HKCU, Key, 'DisplayVersion', Version) and
     (Version <> '') then
    Result := True;

  if RegQueryStringValue(HKCU, Key, 'InstallLocation', InstallLocation) and
     (InstallLocation <> '') then
  begin
    ExePath := AddBackslash(InstallLocation) + 'capin.exe';
    if FileExists(ExePath) then
    begin
      { The executable's fixed version is the most direct record of what
        will actually be replaced; keep the higher of the two values. }
      if GetVersionNumbersString(ExePath, FileVersion) and
         ((not Result) or (CompareVersions(FileVersion, Version) > 0)) then
      begin
        Version := FileVersion;
        Result := True;
      end;
    end
    else if Result then
    begin
      { Orphan uninstall record: do not let it block a fresh install. }
      Version := '';
      Result := False;
    end;
  end;
end;

function InitializeSetup(): Boolean;
var
  InstalledVersion: String;
begin
  Result := True;

  if GetInstalledVersion(InstalledVersion) then
  begin
    if CompareVersions(InstalledVersion, '{#AppVersion}') > 0 then
    begin
      Log('Downgrade blocked: installed=' + InstalledVersion +
        ', installer={#AppVersion}');
      SuppressibleMsgBox(
        'A newer version of Capin (' + InstalledVersion +
        ') is already installed.' + Chr(13) + Chr(10) + Chr(13) + Chr(10) +
        'This installer is version {#AppVersion}. Downgrading is not supported.' +
        Chr(13) + Chr(10) + Chr(13) + Chr(10) +
        'Uninstall the newer version first if you need to install this one.',
        mbError, MB_OK, IDOK);
      Result := False;
      Exit;
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  RunValue: String;
  ExpectedQuoted: String;
  ExpectedPlain: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  if not RegQueryStringValue(HKCU,
    'Software\Microsoft\Windows\CurrentVersion\Run',
    'Capin', RunValue) then
    Exit;

  ExpectedQuoted := '"' + ExpandConstant('{app}\capin.exe') + '"';
  ExpectedPlain := ExpandConstant('{app}\capin.exe');
  if (CompareText(RunValue, ExpectedQuoted) = 0) or
     (CompareText(RunValue, ExpectedPlain) = 0) then
    RegDeleteValue(HKCU,
      'Software\Microsoft\Windows\CurrentVersion\Run',
      'Capin');
end;
