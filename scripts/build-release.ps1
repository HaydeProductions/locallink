$ErrorActionPreference = "Stop"

$Root = Split-Path -Parent $PSScriptRoot
$Dist = Join-Path $Root "dist\LocalLink"
$DefaultAddonsRoot = Join-Path $Root "addons"
$UserAddonsRoot = Join-Path ([Environment]::GetFolderPath("ApplicationData")) "LocalLink\addons"

function Get-AddonManifests {
    if (!(Test-Path $DefaultAddonsRoot)) {
        throw "Add-ons folder not found: $DefaultAddonsRoot"
    }

    $manifests = @(Get-ChildItem -Path $DefaultAddonsRoot -Directory |
        ForEach-Object { Join-Path $_.FullName "manifest.json" } |
        Where-Object { Test-Path $_ })

    if ($manifests.Count -eq 0) {
        throw "No add-on manifests found under $DefaultAddonsRoot"
    }

    return $manifests
}

function Install-AddonFromManifest($ManifestPath, $DestRoot) {
    $manifest = Get-Content $ManifestPath -Raw | ConvertFrom-Json
    $id = [string]$manifest.id
    $exe = [string]$manifest.executable

    if ([string]::IsNullOrWhiteSpace($id) -or [string]::IsNullOrWhiteSpace($exe)) {
        throw "Add-on manifest must contain id and executable: $ManifestPath"
    }

    $builtExe = Join-Path $Root "target\release\$exe"
    if (!(Test-Path $builtExe)) {
        throw "Built add-on binary not found: $builtExe"
    }

    $addonDir = Join-Path $DestRoot $id
    New-Item -ItemType Directory -Force -Path $addonDir | Out-Null
    Copy-Item $builtExe (Join-Path $addonDir $exe) -Force
    Copy-Item $ManifestPath (Join-Path $addonDir "manifest.json") -Force

    Write-Host "Installed add-on $id to:"
    Write-Host $addonDir
}

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
New-Item -ItemType Directory -Force -Path (Join-Path $Dist "addons") | Out-Null
New-Item -ItemType Directory -Force -Path $UserAddonsRoot | Out-Null

Copy-Item (Join-Path $Root "target\release\locallink-core.exe") (Join-Path $Dist "locallink-core.exe") -Force
Copy-Item (Join-Path $Root "target\release\locallink-ui.exe") (Join-Path $Dist "LocalLink.exe") -Force
if (Test-Path (Join-Path $Root "target\release\locallink-tray.exe")) {
    Copy-Item (Join-Path $Root "target\release\locallink-tray.exe") (Join-Path $Dist "LocalLinkTray.exe") -Force
}

foreach ($manifestPath in Get-AddonManifests) {
    Install-AddonFromManifest $manifestPath (Join-Path $Dist "addons")
    Install-AddonFromManifest $manifestPath $UserAddonsRoot
}

@"
LocalLink development package

Run UI:
  .\LocalLink.exe

Run core directly:
  .\locallink-core.exe

Default/debug add-ons are packaged in:
  .\addons

The release script also installs discovered add-ons into:
  %APPDATA%\LocalLink\addons

Space probe logs:
  %LOCALAPPDATA%\LocalLink\logs\space-probe-*.log
  or %APPDATA%\LocalLink\logs\space-probe-*.log

AppData:
  %APPDATA%\LocalLink
"@ | Set-Content (Join-Path $Dist "README.txt")

Write-Host "Built package at:"
Write-Host $Dist
Write-Host "Updated add-ons under:"
Write-Host $UserAddonsRoot
