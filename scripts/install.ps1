#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Builds and installs the native Own Domains app, then applies host setup.

.DESCRIPTION
  Windows counterpart of scripts/install.sh. Builds the desktop app and CLI,
  runs the per-user NSIS installer, previews host setup, then applies it once
  from an elevated prompt (UAC). Windows has no sudo: only the setup apply
  needs elevation, and the GUI itself always runs unprivileged.

.PARAMETER LanAddress
  Reserved LAN IPv4 address of this machine, for example 192.168.1.20.

.PARAMETER Interface
  Optional network interface hint passed to setup.

.PARAMETER DryRun
  Build only the CLI, preview setup, and change nothing.

.EXAMPLE
  ./scripts/install.ps1 192.168.1.20

.EXAMPLE
  ./scripts/install.ps1 -DryRun 192.168.1.20
#>
[CmdletBinding()]
param(
  [Parameter(Position = 0, Mandatory = $true)]
  [string]$LanAddress,
  [string]$Interface = '',
  [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Invoke-OrFail {
  param([string]$Failure, [scriptblock]$Action)
  $global:LASTEXITCODE = 0
  & $Action
  if ($LASTEXITCODE -ne 0) {
    throw $Failure
  }
}

function Invoke-Elevated {
  param([string]$FilePath, [string[]]$Arguments)
  $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Verb RunAs -Wait -PassThru
  if ($process.ExitCode -ne 0) {
    throw "Privileged step failed with exit code $($process.ExitCode): $FilePath $($Arguments -join ' ')"
  }
}

if ($LanAddress -notmatch '^\d{1,3}(\.\d{1,3}){3}$') {
  throw "LAN address must be an IPv4 address, got '$LanAddress'."
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
Push-Location $repositoryRoot
try {
  $cli = Join-Path $repositoryRoot 'target\debug\own-domains.exe'

  if ($DryRun) {
    Write-Host 'BUILD native CLI for the setup preview'
    Invoke-OrFail 'CLI build failed; no system changes were made.' { cargo build -p own-domains-cli }
  }
  else {
    Write-Host 'INSTALL locked dependencies'
    Invoke-OrFail 'dependency installation failed; no system changes were made.' { pnpm install --frozen-lockfile }

    Write-Host 'FETCH pinned sidecars'
    Invoke-OrFail 'sidecar download failed; no system changes were made.' { pnpm run sidecars:fetch }
    Invoke-OrFail 'sidecar download failed; no system changes were made.' { pnpm run sidecars:fetch -- --platform windows }
    Invoke-OrFail 'sidecar download failed; no system changes were made.' { pnpm run sidecars:fetch -- --platform linux }

    Write-Host 'BUILD desktop app (desktop:build:windows)'
    Invoke-OrFail 'desktop build failed; no system changes were made.' { pnpm run desktop:build:windows }

    $cli = Join-Path $repositoryRoot 'target\x86_64-pc-windows-msvc\release\own-domains.exe'
  }

  if (-not (Test-Path -LiteralPath $cli)) {
    throw "Built CLI not found at $cli"
  }

  $setupArguments = @('setup', '--lan-address', $LanAddress)
  if ($Interface) {
    $setupArguments += @('--interface', $Interface)
  }

  Write-Host 'PREVIEW privileged setup'
  Invoke-OrFail 'setup validation failed; no system changes were made.' { & $cli @setupArguments --dry-run }

  if ($DryRun) {
    Write-Host 'Dry run complete; no system changes were made.'
    return
  }

  $bundleDirectory = Join-Path $repositoryRoot 'target\x86_64-pc-windows-msvc\release\bundle\nsis'
  $installer = Get-ChildItem -LiteralPath $bundleDirectory -Filter '*-setup.exe' -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $installer) {
    throw "No NSIS installer found in $bundleDirectory"
  }

  Write-Host "INSTALL $($installer.FullName) (per-user, no elevation)"
  $installProcess = Start-Process -FilePath $installer.FullName -Wait -PassThru
  if ($installProcess.ExitCode -ne 0) {
    throw "Installer failed with exit code $($installProcess.ExitCode)."
  }

  Write-Host 'APPLY privileged setup (UAC approval required)'
  Invoke-Elevated -FilePath $cli -Arguments $setupArguments

  Write-Host 'Install complete. Open Own Domains and press Start.'
}
finally {
  Pop-Location
}
