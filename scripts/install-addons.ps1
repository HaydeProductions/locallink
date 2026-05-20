$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$DistAddons = Join-Path $Root "dist\LocalLink\addons"
$AppDataAddons = Join-Path $env:APPDATA "LocalLink\addons"

if (!(Test-Path $DistAddons)) {
    Write-Host "Dist add-ons not found. Run scripts\build-release.ps1 first."
    exit 1
}

New-Item -ItemType Directory -Force -Path $AppDataAddons | Out-Null
Copy-Item (Join-Path $DistAddons "*") $AppDataAddons -Recurse -Force

Write-Host "Installed add-ons to:"
Write-Host $AppDataAddons
