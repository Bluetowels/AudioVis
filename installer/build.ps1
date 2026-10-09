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

# The installer shows the README when it finishes. Markdown opens as plain
# text in a browser on most PCs, so it ships as a styled HTML page instead.
if (-not (Get-Command ConvertFrom-Markdown -ErrorAction SilentlyContinue)) {
    throw "This script needs PowerShell 7 (pwsh) for ConvertFrom-Markdown. Install it with: winget install Microsoft.PowerShell"
}
# The installed guide is for people using the app: parts of the README marked
# source-only are left out, and installed-only notes are shown.
$guide = Get-Content "$root\README.md" -Raw
$guide = [regex]::Replace($guide, '(?s)<!-- source-only -->.*?<!-- /source-only -->\r?\n?', '')
$guide = [regex]::Replace($guide, '<!-- installed-only: (.*?) -->', '$1')
$body = ($guide | ConvertFrom-Markdown).Html
$style = @"
:root { color-scheme: dark; }
body { background: #15151a; color: #dcdce2; font: 16px/1.6 "Segoe UI", system-ui, sans-serif; margin: 0; }
main { max-width: 860px; margin: 0 auto; padding: 32px 24px 64px; }
h1 { font-size: 2.2em; margin: 0 0 0.4em; color: #fff; }
h2 { margin-top: 2em; padding-bottom: 0.3em; border-bottom: 1px solid #33333c; color: #fff; }
h3 { margin-top: 1.6em; color: #fff; }
a { color: #5ac8ff; }
code { background: #24242c; padding: 0.1em 0.35em; border-radius: 4px; font: 0.9em Consolas, monospace; }
pre { background: #24242c; padding: 12px 16px; border-radius: 6px; overflow-x: auto; }
pre code { padding: 0; background: none; }
table { border-collapse: collapse; margin: 1em 0; width: 100%; }
th, td { border: 1px solid #33333c; padding: 6px 10px; text-align: left; vertical-align: top; }
th { background: #1e1e25; }
li { margin: 0.25em 0; }
"@
$html = "<!doctype html>`n<html lang=`"en`">`n<head>`n<meta charset=`"utf-8`">`n<meta name=`"viewport`" content=`"width=device-width, initial-scale=1`">`n<title>AudioVis $version</title>`n<style>`n$style</style>`n</head>`n<body>`n<main>`n$body</main>`n</body>`n</html>`n"
New-Item -ItemType Directory -Force "$PSScriptRoot\output" | Out-Null
Set-Content -Path "$PSScriptRoot\output\README.html" -Value $html -Encoding utf8

$iscc = @("${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }

& $iscc "/DAppVersion=$version" "/DExePath=$target\release\audiovis.exe" "$PSScriptRoot\audiovis.iss"
if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
Write-Host "Installer: $PSScriptRoot\output\AudioVis-Setup-$version.exe"
