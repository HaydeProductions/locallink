$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root "dist\LocalLink"
$UserSpaceProbeDir = Join-Path ([Environment]::GetFolderPath("ApplicationData")) "LocalLink\addons\space-probe"

Write-Host "Building LocalLink workspace release..."
Push-Location $Root
cargo build --release
if ($LASTEXITCODE -ne 0) {
    Pop-Location
    throw "cargo build --release failed"
}
Pop-Location

if (Test-Path $Dist) {
    Remove-Item $Dist -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $Dist | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Dist "addons\clipboard-sync") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $Dist "addons\space-probe") | Out-Null

Copy-Item (Join-Path $Root "target\release\locallink-core.exe") (Join-Path $Dist "locallink-core.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-ui.exe") (Join-Path $Dist "LocalLink.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-addon-clipboard.exe") (Join-Path $Dist "addons\clipboard-sync\locallink-addon-clipboard.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-addon-space-probe.exe") (Join-Path $Dist "addons\space-probe\locallink-addon-space-probe.exe") -Force

@"
{
  "id": "clipboard-sync",
  "name": "Clipboard Sync",
  "version": "0.1.0",
  "description": "Keeps text clipboards synchronized across connected LocalLink devices. Newest clipboard wins.",
  "executable": "locallink-addon-clipboard.exe",
  "services": [
    "clipboard-sync"
  ],
  "enabled": false
}
"@ | Set-Content (Join-Path $Dist "addons\clipboard-sync\manifest.json")

Copy-Item (Join-Path $Root "locallink-addon-space-probe\manifest.json") (Join-Path $Dist "addons\space-probe\manifest.json") -Force

New-Item -ItemType Directory -Force -Path $UserSpaceProbeDir | Out-Null
Copy-Item (Join-Path $Root "target\release\locallink-addon-space-probe.exe") (Join-Path $UserSpaceProbeDir "locallink-addon-space-probe.exe") -Force
Copy-Item (Join-Path $Root "locallink-addon-space-probe\manifest.json") (Join-Path $UserSpaceProbeDir "manifest.json") -Force
Write-Host "Installed debug add-on space-probe to:"
Write-Host $UserSpaceProbeDir

@"
LocalLink development package

Run UI:
  .\LocalLink.exe

Run core directly:
  .\locallink-core.exe

Run clipboard sync addon:
  .\addons\clipboard-sync\locallink-addon-clipboard.exe

Run space probe debug addon:
  .\addons\space-probe\locallink-addon-space-probe.exe

Space probe logs:
  %LOCALAPPDATA%\LocalLink\logs\space-probe-*.log
  or %APPDATA%\LocalLink\logs\space-probe-*.log

AppData:
  %APPDATA%\LocalLink
"@ | Set-Content (Join-Path $Dist "README.txt")

Write-Host "Built package at:"
Write-Host $Dist
