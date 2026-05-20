$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root "dist\LocalLink"

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

Copy-Item (Join-Path $Root "target\release\locallink-core.exe") (Join-Path $Dist "locallink-core.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-ui.exe") (Join-Path $Dist "LocalLink.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-addon-clipboard.exe") (Join-Path $Dist "addons\clipboard-sync\locallink-addon-clipboard.exe") -Force

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

@"
LocalLink development package

Run UI:
  .\LocalLink.exe

Run core directly:
  .\locallink-core.exe

Run clipboard sync addon:
  .\addons\clipboard-sync\locallink-addon-clipboard.exe

AppData:
  %APPDATA%\LocalLink
"@ | Set-Content (Join-Path $Dist "README.txt")

Write-Host "Built package at:"
Write-Host $Dist
