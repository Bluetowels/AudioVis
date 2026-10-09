; Inno Setup script for AudioVis. Build with installer\build.ps1, which
; compiles the release exe, fetches the VC++ redistributable and runs ISCC.

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef ExePath
  #define ExePath "D:\cargo-target\audiovis\release\audiovis.exe"
#endif

[Setup]
AppId={{6C1E2F4A-8B3D-4E7A-9F21-3A5D7C9B0E14}
AppName=AudioVis
AppVersion={#AppVersion}
AppPublisher=Kieron
DefaultDirName={autopf}\AudioVis
DefaultGroupName=AudioVis
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\audiovis.exe
OutputDir=output
OutputBaseFilename=AudioVis-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
PrivilegesRequired=admin

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion isreadme
Source: "redist\vc_redist.x64.exe"; DestDir: "{tmp}"; Flags: deleteafterinstall; Check: VCRedistNeeded

[Icons]
Name: "{group}\AudioVis"; Filename: "{app}\audiovis.exe"
Name: "{group}\Uninstall AudioVis"; Filename: "{uninstallexe}"
Name: "{autodesktop}\AudioVis"; Filename: "{app}\audiovis.exe"; Tasks: desktopicon

[Run]
Filename: "{tmp}\vc_redist.x64.exe"; Parameters: "/install /quiet /norestart"; StatusMsg: "Installing the Microsoft Visual C++ runtime..."; Check: VCRedistNeeded; Flags: waituntilterminated
Filename: "{app}\audiovis.exe"; Description: "{cm:LaunchProgram,AudioVis}"; Flags: nowait postinstall skipifsilent

[Code]
// The Visual C++ 2015-2022 runtime records itself here when installed.
function VCRedistNeeded: Boolean;
var
  Installed: Cardinal;
begin
  Result := not (RegQueryDWordValue(HKLM64, 'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64', 'Installed', Installed) and (Installed = 1));
end;

// Every Vulkan-capable graphics driver installs the Vulkan loader.
function InitializeSetup: Boolean;
begin
  Result := True;
  if not FileExists(ExpandConstant('{sys}\vulkan-1.dll')) then
  begin
    MsgBox('AudioVis needs a graphics card driver with Vulkan support, and none was found on this PC.' + #13#10#13#10 +
      'Please install the latest driver from AMD, NVIDIA or Intel, then run this installer again.', mbCriticalError, MB_OK);
    Result := False;
  end;
end;
