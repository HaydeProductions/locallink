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
New-Item -ItemType Directory -Force -Path (Join-Path $Dist "addons\example-echo") | Out-Null

Copy-Item (Join-Path $Root "target\release\locallink-core.exe") (Join-Path $Dist "locallink-core.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-ui.exe") (Join-Path $Dist "LocalLink.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-addon-echo.exe") (Join-Path $Dist "addons\example-echo\locallink-addon-echo.exe") -Force

@"
{
  "id": "example-echo",
  "name": "Example Echo Addon",
  "version": "0.1.0",
  "description": "Simple addon that listens on test.echo and replies on test.echo.reply.",
  "executable": "locallink-addon-echo.exe",
  "services": [
    "test.echo",
    "test.echo.reply"
  ],
  "enabled": true
}
"@ | Set-Content (Join-Path $Dist "addons\example-echo\manifest.json")

@"
LocalLink development package

Run UI:
  .\LocalLink.exe

Run core directly:
  .\locallink-core.exe

API helper:
  .\locallink-core.exe --api status
  .\locallink-core.exe --api paths
  .\locallink-core.exe --api addons
  .\locallink-core.exe --api shutdown

Run example addon directly:
  .\addons\example-echo\locallink-addon-echo.exe

AppData:
  %APPDATA%\LocalLink
"@ | Set-Content (Join-Path $Dist "README.txt")

Write-Host "Built package at:"
Write-Host $Dist
