$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$DistAddon = Join-Path $Root "dist\LocalLink\addons\example-echo"
$AppDataAddon = Join-Path $env:APPDATA "LocalLink\addons\example-echo"

if (!(Test-Path $DistAddon)) {
    Write-Host "Dist addon not found. Run scripts\build-release.ps1 first."
    exit 1
}

New-Item -ItemType Directory -Force -Path $AppDataAddon | Out-Null
Copy-Item (Join-Path $DistAddon "locallink-addon-echo.exe") (Join-Path $AppDataAddon "locallink-addon-echo.exe") -Force
Copy-Item (Join-Path $DistAddon "manifest.json") (Join-Path $AppDataAddon "manifest.json") -Force

Write-Host "Installed example addon to:"
Write-Host $AppDataAddon
