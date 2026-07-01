param(
    [switch]$Follow,
    [switch]$Raw,
    [int]$Tail = 160,
    [string]$Contains = ""
)

$ErrorActionPreference = "Stop"

$LogRoot = Join-Path ([Environment]::GetFolderPath("ApplicationData")) "LocalLink\logs"

if (!(Test-Path $LogRoot)) {
    Write-Host "LocalLink log folder does not exist yet:"
    Write-Host $LogRoot
    exit 0
}

function Show-Line {
    param([string]$Line)

    if ($Raw) { return $true }
    if ($Contains -ne "") { return $Line -like "*$Contains*" }

    return (
        $Line -like "*SetSpaceAddonEnabled*" -or
        $Line -like "*set_space_addon_enabled*" -or
        $Line -like "*ActivateSpace*" -or
        $Line -like "*activate_space*" -or
        $Line -like "*ok=false*" -or
        $Line -like "*error=*" -or
        $Line -like "*failed*" -or
        $Line -like "*addon-manager*" -or
        $Line -like "*addon-launch*" -or
        $Line -like "*space-probe*" -or
        $Line -like "*send_space_message*"
    )
}

function Label-Line {
    param([string]$Line)

    if ($Raw) { return $Line }

    if ($Line -like "*SetSpaceAddonEnabled*") { return "UI ACTION    $Line" }
    if ($Line -like "*[ui-api]*request*") { return "UI TO CORE   $Line" }
    if ($Line -like "*[ui-api]*response*") { return "CORE TO UI   $Line" }
    if ($Line -like "*addon-manager*") { return "ADDON PLAN   $Line" }
    if ($Line -like "*addon-launch*") { return "ADDON START  $Line" }
    if ($Line -like "*space-probe*") { return "SPACE PROBE  $Line" }
    if ($Line -like "*ok=false*" -or $Line -like "*error=*" -or $Line -like "*failed*") { return "ERROR        $Line" }

    return "DEBUG        $Line"
}

$files = @()
foreach ($pattern in @("diagnostics.log", "ui-process-*.log", "dev-launch-*.log", "space-probe-*.log")) {
    $files += Get-ChildItem -Path $LogRoot -Filter $pattern -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 3
}

$files = $files | Sort-Object LastWriteTime -Descending -Unique

Write-Host "LocalLink organised debugger"
Write-Host "Logs: $LogRoot"
Write-Host "Mode: $(if ($Raw) { 'raw' } elseif ($Contains -ne '') { "contains $Contains" } else { 'actions only' })"
Write-Host "Use -Raw for every line, or -Contains clipboard-sync to focus on a term."
Write-Host ""

if ($files.Count -eq 0) {
    Write-Host "No log files found yet. Run git launch first."
    exit 0
}

if ($Follow) {
    Write-Host "Following. Press Ctrl+C to stop."
    Write-Host ""
    Get-Content -Path ($files.FullName) -Tail $Tail -Wait |
        Where-Object { Show-Line $_ } |
        ForEach-Object { Label-Line $_ }
} else {
    foreach ($file in $files) {
        $lines = Get-Content -Path $file.FullName -Tail $Tail | Where-Object { Show-Line $_ }
        if ($lines.Count -eq 0) { continue }

        Write-Host "==== $($file.Name) ===="
        $lines | ForEach-Object { Label-Line $_ }
        Write-Host ""
    }
}
