# LocalLink Windows network diagnostics
# Run from PowerShell. Administrator is not required for checks.

param(
    [string]$InterfaceAlias = ""
)

$ErrorActionPreference = "Continue"

function Write-Section($Title) {
    Write-Host ""
    Write-Host "=== $Title ===" -ForegroundColor Cyan
}

function Get-TargetAdapters {
    if ($InterfaceAlias.Trim().Length -gt 0) {
        return @(Get-NetAdapter -Name $InterfaceAlias -ErrorAction Stop)
    }

    return @(Get-NetAdapter | Where-Object { $_.Status -eq "Up" } | Sort-Object Name)
}

Write-Section "LocalLink network check"
Write-Host "Discovery UDP: 47777"
Write-Host "Core TCP:      47800"
Write-Host "Local API:     47900 (localhost only)"

Write-Section "Active adapters"
$adapters = Get-TargetAdapters
$adapters | Select-Object Name, ifIndex, Status, MacAddress, InterfaceDescription | Format-Table -AutoSize

Write-Section "IPv6 binding"
$ipv6Rows = foreach ($adapter in $adapters) {
    $binding = Get-NetAdapterBinding -Name $adapter.Name -ComponentID ms_tcpip6 -ErrorAction SilentlyContinue
    [PSCustomObject]@{
        Name = $adapter.Name
        ifIndex = $adapter.ifIndex
        IPv6Enabled = if ($binding) { $binding.Enabled } else { "Unknown" }
    }
}
$ipv6Rows | Format-Table -AutoSize

Write-Section "Network profile"
Get-NetConnectionProfile | Select-Object Name, InterfaceAlias, InterfaceIndex, NetworkCategory | Format-Table -AutoSize

Write-Section "LocalLink firewall rules"
$rules = Get-NetFirewallRule -DisplayName "LocalLink*" -ErrorAction SilentlyContinue
if ($rules) {
    $rules | Select-Object DisplayName, Enabled, Direction, Action, Profile | Format-Table -AutoSize

    $portRows = foreach ($rule in $rules) {
        Get-NetFirewallPortFilter -AssociatedNetFirewallRule $rule -ErrorAction SilentlyContinue |
            Select-Object @{Name="Rule"; Expression={$rule.DisplayName}}, Protocol, LocalPort
    }

    if ($portRows) {
        $portRows | Format-Table -AutoSize
    }
} else {
    Write-Host "No LocalLink firewall rules found."
}

Write-Section "Ports currently listening"
Get-NetUDPEndpoint -LocalPort 47777 -ErrorAction SilentlyContinue | Select-Object LocalAddress, LocalPort, OwningProcess | Format-Table -AutoSize
Get-NetTCPConnection -LocalPort 47800,47900 -State Listen -ErrorAction SilentlyContinue | Select-Object LocalAddress, LocalPort, State, OwningProcess | Format-Table -AutoSize

Write-Section "Suggested next step"
Write-Host "To apply the standard LocalLink firewall and IPv6 setup, run PowerShell as Administrator:"
Write-Host "  ./scripts/windows-network-setup.ps1"
Write-Host ""
Write-Host "To target one adapter only:"
Write-Host "  ./scripts/windows-network-setup.ps1 -InterfaceAlias \"Ethernet\""
