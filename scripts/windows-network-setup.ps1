# LocalLink Windows network setup
# Run PowerShell as Administrator.
# This script enables IPv6 on selected adapters, sets active profiles to Private,
# and creates inbound firewall rules for LocalLink discovery/core traffic.

param(
    [string]$InterfaceAlias = "",
    [switch]$SkipPrivateProfile
)

$ErrorActionPreference = "Stop"

function Assert-Admin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw "This script must be run as Administrator. Right-click PowerShell and choose 'Run as administrator'."
    }
}

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

Assert-Admin

Write-Section "Selecting adapters"
$adapters = Get-TargetAdapters
if ($adapters.Count -eq 0) {
    throw "No matching active adapters found."
}
$adapters | Select-Object Name, ifIndex, Status, InterfaceDescription | Format-Table -AutoSize

Write-Section "Enabling IPv6 bindings"
foreach ($adapter in $adapters) {
    $binding = Get-NetAdapterBinding -Name $adapter.Name -ComponentID ms_tcpip6 -ErrorAction SilentlyContinue
    if ($binding -and -not $binding.Enabled) {
        Enable-NetAdapterBinding -Name $adapter.Name -ComponentID ms_tcpip6
        Write-Host "Enabled IPv6 on $($adapter.Name)"
    } elseif ($binding) {
        Write-Host "IPv6 already enabled on $($adapter.Name)"
    } else {
        Write-Warning "Could not inspect IPv6 binding on $($adapter.Name)"
    }
}

if (-not $SkipPrivateProfile) {
    Write-Section "Setting active network profiles to Private"
    foreach ($adapter in $adapters) {
        $profiles = @(Get-NetConnectionProfile -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue)
        foreach ($profile in $profiles) {
            if ($profile.NetworkCategory -ne "Private") {
                Set-NetConnectionProfile -InterfaceIndex $adapter.ifIndex -NetworkCategory Private
                Write-Host "Set $($profile.InterfaceAlias) to Private"
            } else {
                Write-Host "$($profile.InterfaceAlias) already Private"
            }
        }
    }
}

Write-Section "Creating firewall rules"
$ruleNames = @(
    "LocalLink UDP Discovery",
    "LocalLink TCP Core",
    "LocalLink ICMPv6"
)

foreach ($ruleName in $ruleNames) {
    Remove-NetFirewallRule -DisplayName $ruleName -ErrorAction SilentlyContinue
}

New-NetFirewallRule `
    -DisplayName "LocalLink UDP Discovery" `
    -Direction Inbound `
    -Action Allow `
    -Protocol UDP `
    -LocalPort 47777 `
    -Profile Any | Out-Null

New-NetFirewallRule `
    -DisplayName "LocalLink TCP Core" `
    -Direction Inbound `
    -Action Allow `
    -Protocol TCP `
    -LocalPort 47800 `
    -Profile Any | Out-Null

New-NetFirewallRule `
    -DisplayName "LocalLink ICMPv6" `
    -Direction Inbound `
    -Action Allow `
    -Protocol ICMPv6 `
    -Profile Any | Out-Null

Write-Section "Done"
Write-Host "LocalLink network setup complete."
Write-Host "Restart LocalLink Core after running this script."
Write-Host ""
Write-Host "Check current state with:"
Write-Host "  ./scripts/windows-network-check.ps1"
