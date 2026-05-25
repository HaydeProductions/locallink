# LocalLink Windows network repair launcher
# This script checks the basics without requiring admin. If a fix is needed,
# it launches the admin setup script and lets Windows show the UAC Allow prompt.

param(
    [string]$InterfaceAlias = ""
)

$ErrorActionPreference = "Continue"

function Test-IsAdmin {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = New-Object Security.Principal.WindowsPrincipal($identity)
    return $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-TargetAdapters {
    if ($InterfaceAlias.Trim().Length -gt 0) {
        return @(Get-NetAdapter -Name $InterfaceAlias -ErrorAction Stop)
    }

    return @(Get-NetAdapter | Where-Object { $_.Status -eq "Up" } | Sort-Object Name)
}

function Quote-Arg([string]$Value) {
    return '"' + $Value.Replace('"', '\"') + '"'
}

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$setupScript = Join-Path $scriptDir "windows-network-setup.ps1"

if (-not (Test-Path $setupScript)) {
    Write-Error "Could not find setup script: $setupScript"
    exit 1
}

$needsFix = $false
$reasons = New-Object System.Collections.Generic.List[string]

try {
    $adapters = Get-TargetAdapters
    if ($adapters.Count -eq 0) {
        $needsFix = $true
        $reasons.Add("No active network adapters were found.")
    }

    foreach ($adapter in $adapters) {
        $binding = Get-NetAdapterBinding -Name $adapter.Name -ComponentID ms_tcpip6 -ErrorAction SilentlyContinue
        if (-not $binding) {
            $needsFix = $true
            $reasons.Add("Could not inspect IPv6 binding on $($adapter.Name).")
        } elseif (-not $binding.Enabled) {
            $needsFix = $true
            $reasons.Add("IPv6 is disabled on $($adapter.Name).")
        }

        $profiles = @(Get-NetConnectionProfile -InterfaceIndex $adapter.ifIndex -ErrorAction SilentlyContinue)
        foreach ($profile in $profiles) {
            if ($profile.NetworkCategory -ne "Private") {
                $needsFix = $true
                $reasons.Add("Network profile for $($profile.InterfaceAlias) is $($profile.NetworkCategory), not Private.")
            }
        }
    }

    $requiredRules = @(
        @{ Name = "LocalLink UDP Discovery"; Protocol = "UDP"; Port = "47777" },
        @{ Name = "LocalLink TCP Core"; Protocol = "TCP"; Port = "47800" },
        @{ Name = "LocalLink ICMPv6"; Protocol = "ICMPv6"; Port = "Any" }
    )

    foreach ($required in $requiredRules) {
        $rule = Get-NetFirewallRule -DisplayName $required.Name -ErrorAction SilentlyContinue
        if (-not $rule) {
            $needsFix = $true
            $reasons.Add("Firewall rule missing: $($required.Name).")
            continue
        }

        if ($rule.Enabled -ne "True" -or $rule.Action -ne "Allow" -or $rule.Direction -ne "Inbound") {
            $needsFix = $true
            $reasons.Add("Firewall rule is not enabled/allow/inbound: $($required.Name).")
        }
    }
} catch {
    $needsFix = $true
    $reasons.Add("Check failed: $($_.Exception.Message)")
}

if (-not $needsFix) {
    Write-Host "LocalLink network requirements look OK."
    exit 0
}

Write-Host "LocalLink found network requirements that need attention:"
foreach ($reason in $reasons) {
    Write-Host " - $reason"
}

$argumentList = "-NoProfile -ExecutionPolicy Bypass -File " + (Quote-Arg $setupScript)
if ($InterfaceAlias.Trim().Length -gt 0) {
    $argumentList += " -InterfaceAlias " + (Quote-Arg $InterfaceAlias)
}

if (Test-IsAdmin) {
    & $setupScript @PSBoundParameters
    exit $LASTEXITCODE
}

Write-Host "Requesting Windows administrator permission to apply fixes..."
Start-Process powershell.exe -Verb RunAs -ArgumentList $argumentList -Wait
exit $LASTEXITCODE
