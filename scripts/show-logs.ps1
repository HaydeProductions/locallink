param(
    [switch]$Follow,
    [int]$Tail = 120
)

$ErrorActionPreference = "Stop"

$LogRoot = Join-Path ([Environment]::GetFolderPath("ApplicationData")) "LocalLink\logs"

if (!(Test-Path $LogRoot)) {
    Write-Host "LocalLink log folder does not exist yet:"
    Write-Host $LogRoot
    exit 0
}

Write-Host "LocalLink logs: $LogRoot"
Write-Host ""

$interesting = @(
    "diagnostics.log",
    "ui-process-*.log",
    "dev-launch-*.log",
    "space-probe-*.log"
)

$files = @()
foreach ($pattern in $interesting) {
    $files += Get-ChildItem -Path $LogRoot -Filter $pattern -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 3
}

$files = $files | Sort-Object LastWriteTime -Descending -Unique

if ($files.Count -eq 0) {
    Write-Host "No diagnostics logs found yet. Run git launch, start the UI, then click the action you want to debug."
    exit 0
}

Write-Host "Recent diagnostic files:"
$files | ForEach-Object {
    Write-Host ("  {0}  {1}" -f $_.LastWriteTime.ToString("yyyy-MM-dd HH:mm:ss"), $_.FullName)
}
Write-Host ""

if ($Follow) {
    Write-Host "Following logs. Press Ctrl+C to stop."
    Get-Content -Path ($files.FullName) -Tail $Tail -Wait
} else {
    foreach ($file in $files) {
        Write-Host "==== $($file.Name) ===="
        Get-Content -Path $file.FullName -Tail $Tail
        Write-Host ""
    }
}
