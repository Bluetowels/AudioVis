# Builds the release exe and the installer: installer\output\AudioVis-Setup-<version>.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent

$version = (Select-String -Path "$root\visualiser\Cargo.toml" -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$root\visualiser\target" }

Push-Location "$root\visualiser"
cargo build --release
Pop-Location
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

$redist = "$PSScriptRoot\redist\vc_redist.x64.exe"
if (-not (Test-Path $redist)) {
    New-Item -ItemType Directory -Force "$PSScriptRoot\redist" | Out-Null
    Invoke-WebRequest 'https://aka.ms/vs/17/release/vc_redist.x64.exe' -OutFile $redist
}

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }

& $iscc "/DAppVersion=$version" "/DExePath=$target\release\audiovis.exe" "$PSScriptRoot\audiovis.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
Write-Host "Installer: $PSScriptRoot\output\AudioVis-Setup-$version.exe"
