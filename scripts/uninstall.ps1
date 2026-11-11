#!/usr/bin/env pwsh
<#
.SYNOPSIS
  Removes the native Own Domains host integration and the installed app.

.DESCRIPTION
  Windows counterpart of scripts/uninstall.sh. Removes the managed NRPT rule,
  firewall rule, and per-user CA from an elevated prompt (UAC), then runs the
  NSIS uninstaller. Independent steps continue after an error and every
  failure is reported with a nonzero exit code.

.PARAMETER DryRun
  Preview the host-integration removal and change nothing.

.PARAMETER Purge
  Also remove settings, backups, logs, and the local CA data.
#>
[CmdletBinding()]
param(
  [switch]$DryRun,
  [switch]$Purge
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if ($DryRun -and $Purge) {
  throw 'Purge cannot be combined with DryRun.'
}

$errors = [System.Collections.Generic.List[string]]::new()

function Invoke-Step {
  param([string]$Failure, [scriptblock]$Action)
  try {
    $global:LASTEXITCODE = 0
    & $Action
    if ($LASTEXITCODE -ne 0) {
      throw "exit code $LASTEXITCODE"
    }
  }
  catch {
    $message = "$Failure ($($_.Exception.Message))"
    Write-Host "ERROR: $message" -ForegroundColor Red
    $errors.Add($message)
  }
}

function Invoke-Elevated {
  param([string]$FilePath, [string[]]$Arguments)
  $process = Start-Process -FilePath $FilePath -ArgumentList $Arguments -Verb RunAs -Wait -PassThru
  if ($process.ExitCode -ne 0) {
    throw "exit code $($process.ExitCode)"
  }
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$appDirectory = Join-Path $env:LOCALAPPDATA 'Own Domains'
Push-Location $repositoryRoot
try {
  $cli = $null
  $command = Get-Command 'own-domains.exe' -ErrorAction SilentlyContinue
  if ($command) {
    $cli = $command.Source
  }
  elseif (Test-Path -LiteralPath (Join-Path $appDirectory 'own-domains.exe')) {
    $cli = Join-Path $appDirectory 'own-domains.exe'
  }
  else {
    $debugCli = Join-Path $repositoryRoot 'target\debug\own-domains.exe'
    if (-not (Test-Path -LiteralPath $debugCli)) {
      Write-Host 'BUILD native CLI'
      & cargo build -p own-domains-cli
      if ($LASTEXITCODE -ne 0) {
        throw 'CLI build failed; nothing was removed'
      }
    }
    $cli = $debugCli
  }

  if ($DryRun) {
    Write-Host 'PREVIEW host integration removal'
    & $cli uninstall --dry-run
    exit $LASTEXITCODE
  }

  $uninstallArguments = @('uninstall')
  if ($Purge) {
    $uninstallArguments += '--purge'
  }

  Write-Host 'UNINSTALL managed host integration (UAC approval required)'
  Invoke-Step 'managed-file uninstall failed' { Invoke-Elevated -FilePath $cli -Arguments $uninstallArguments }

  $uninstaller = Join-Path $appDirectory 'uninstall.exe'
  if (Test-Path -LiteralPath $uninstaller) {
    Write-Host "UNINSTALL app from $appDirectory"
    Invoke-Step 'could not remove the installed app' {
      $process = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
      if ($process.ExitCode -ne 0) {
        throw "exit code $($process.ExitCode)"
      }
    }
  }

  if ($errors.Count -gt 0) {
    Write-Host "Cleanup finished with $($errors.Count) error(s):" -ForegroundColor Red
    foreach ($message in $errors) {
      Write-Host "- $message" -ForegroundColor Red
    }
    exit 1
  }

  Write-Host 'Own Domains removed.'
}
finally {
  Pop-Location
}
