<#
.SYNOPSIS
  Runs a program in the build user's logged-in desktop session (Windows) and
  waits for it: windows, the GPU and VR headsets are only available there,
  not to programs started over SSH (spec §11.2, config/README.md).

.DESCRIPTION
  Creates a one-off scheduled task (schtasks /it) for the current user's
  interactive session, starts it, waits until it ends (or -TimeoutSec), copies
  its output to -Log and removes the task. The user must be logged in on the
  desktop (the screen may be locked); otherwise the task cannot start and the
  script says so.

  Example (over `scripts/remote.sh Weela run`):
    scripts/win-gui-run.ps1 -Exe target\release\ocw-desktop.exe `
      -Arguments '--vr --screenshot test-output\vr.jpg --screenshot-delay 20' `
      -Log test-output\vr-run.log -TimeoutSec 180

.PARAMETER Exe
  The program, relative to the working directory or absolute.

.PARAMETER Arguments
  Its command line (one string).

.PARAMETER Log
  Where its stdout and stderr go (default: test-output\gui-run.log).

.PARAMETER TimeoutSec
  Ends the program after this many seconds (default 300).
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Exe,
    [string]$Arguments = "",
    [string]$Log = "test-output\gui-run.log",
    [int]$TimeoutSec = 300
)
$ErrorActionPreference = "Stop"

$dir = (Get-Location).Path
$exePath = if ([IO.Path]::IsPathRooted($Exe)) { $Exe } else { Join-Path $dir $Exe }
if (-not (Test-Path $exePath)) { throw "not found: $exePath" }
$logPath = if ([IO.Path]::IsPathRooted($Log)) { $Log } else { Join-Path $dir $Log }
New-Item -ItemType Directory -Force (Split-Path $logPath) | Out-Null
Remove-Item $logPath -ErrorAction SilentlyContinue

$me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
# schtasks.exe, not the ScheduledTasks cmdlets: those go through CIM, which
# Windows refuses to a non-admin user over SSH. A .cmd wrapper carries the
# command line (schtasks /tr takes at most 261 characters) and sends stdout
# and stderr to the log; /it runs it only in the user's interactive session.
$name = "ocw-gui-run-$PID"
$wrapper = Join-Path $env:TEMP "$name.cmd"
$done = Join-Path $env:TEMP "$name.exit"
Remove-Item $done -ErrorAction SilentlyContinue
@"
@echo off
cd /d "$dir"
"$exePath" $Arguments > "$logPath" 2>&1
echo %ERRORLEVEL% > "$done"
"@ | Set-Content -Encoding ascii $wrapper
schtasks /create /tn $name /tr "`"$wrapper`"" /sc once /st 00:00 /sd 01/01/2099 /it /f | Out-Null
if ($LASTEXITCODE -ne 0) { throw "schtasks /create failed ($LASTEXITCODE)" }
try {
    schtasks /run /tn $name | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "schtasks /run failed ($LASTEXITCODE)" }
    $t0 = Get-Date
    $started = $false
    while (-not (Test-Path $done)) {
        $secs = ((Get-Date) - $t0).TotalSeconds
        if (-not $started -and (Test-Path $logPath)) { $started = $true }
        if (-not $started -and $secs -gt 20) {
            throw "the task did not start within 20 s: is $me logged in on the desktop? (the screen may stay locked)"
        }
        if ($secs -gt $TimeoutSec) {
            Write-Warning "timeout after $TimeoutSec s: ending it"
            schtasks /end /tn $name | Out-Null
            Get-Process -Name ([IO.Path]::GetFileNameWithoutExtension($exePath)) -ErrorAction SilentlyContinue | Stop-Process -Force
            Start-Sleep -Seconds 2
            break
        }
        Start-Sleep -Seconds 1
    }
    $code = if (Test-Path $done) { [int]((Get-Content $done -Raw).Trim()) } else { 124 }
    if (Test-Path $logPath) { Get-Content $logPath -Tail 60 }
    Write-Host "gui-run: exit $code after $([math]::Round(((Get-Date) - $t0).TotalSeconds)) s, log $Log"
    exit $code
} finally {
    schtasks /delete /tn $name /f 2>$null | Out-Null
    Remove-Item $wrapper, $done -ErrorAction SilentlyContinue
}
