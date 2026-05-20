$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$Exe = Join-Path $Root "dist\LocalLink\locallink-core.exe"

if (!(Test-Path $Exe)) {
    Write-Host "Release exe not found. Run scripts\build-release.ps1 first."
    exit 1
}

& $Exe --api @args
