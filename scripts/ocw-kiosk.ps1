<#
.SYNOPSIS
  The Windows VR kiosk (spec 14 Phase 11, 11I): keeps the OCW test build
  running in VR on a Windows machine with a headset (Weela: Meta Quest over
  Link), like scripts/ocw-kiosk.sh on the Linux kiosks.

.DESCRIPTION
  Run it as the build user (no admin needed):

    ocw-kiosk.ps1 install     register the per-user scheduled task "OCW Kiosk"
                              (started by hand: `start`, the desktop shortcuts
                              "OCW VR Kiosk - Start / Stop") and start it now
                              (-NoStart: not now). **No autostart by default**
                              (the user, 2026-11-18: "stop and disable the
                              kiosk app, don't make it start up by default");
                              -Autostart adds the logon trigger (or the
                              Startup folder entry when the trigger is
                              refused). Installing again without -Autostart
                              removes an earlier autostart.
    ocw-kiosk.ps1 uninstall   stop it and remove the task / Startup entry
    ocw-kiosk.ps1 start | stop | restart
    ocw-kiosk.ps1 status      the deployed build, the kiosk, the VR line, log
    ocw-kiosk.ps1 run         the supervisor (what the task runs): start the
                              deployed ocw-desktop.exe --vr in show mode,
                              restart it when it exits or a new deployment
                              appears; log to the state folder
    ocw-kiosk.ps1 screenshot [-Out FILE]
                              a picture of the monitor (the left eye: the
                              monitor mirrors it) taken in the desktop session
    ocw-kiosk.ps1 deploy -Stage DIR -Version TEXT [-World NAME] [-Route NAME]
                              install a staged build (scripts/deploy-test.sh
                              MACHINE does the staging and calls this)
    ocw-kiosk.ps1 selftest    the unit tests of the pure functions below

  The app runs in the build user's interactive desktop session (windows,
  the GPU and the headset are only there; the scheduled task runs only
  there, see scripts/win-gui-run.ps1): a 960 x 540 window on the monitor (the left
  eye mirrored), VR in the headset. Without a headset (Link off, runtime
  missing) the app runs as before with the message "VR is off: ..."; the
  kiosk logs it and restarts the app every OCW_KIOSK_VR_RETRY_S seconds (60)
  (default 300, 0 = never) to try again. A session lost later reconnects in
  the app by itself.

  Files: the kiosk folder (OCW_KIOSK_DIR, default %LOCALAPPDATA%\OCW\kiosk):
    ocw-kiosk.ps1     this script (a deploy replaces it; the running kiosk
                      then restarts on the new one)
    current.txt       the build folder in use (builds\ID); a deploy writes it
                      last, the kiosk restarts the app on a change
    builds\ID\        ocw-desktop.exe, openxr_loader.dll (+ licence),
                      materials\, ocw-desktop.args (the kiosk's world, show
                      mode, --vr --window 960x540), VERSION
    worlds\           world packages with manifests and the route files
    kiosk-local.args  machine-local arguments (kept, appended)
    deploying         while it exists the kiosk stops the app (a deploy
                      replacing a world package the app holds open)
  The state folder (%LOCALAPPDATA%\OCW\kiosk-state): kiosk.log, app.log (the
  app's log; app.prev.log the previous run's), app.out.log, pid files.

  PowerShell 5.1 (Windows' own) and newer; ASCII only (5.1 reads files
  without a BOM as ANSI).
#>
param(
    [Parameter(Position = 0)][string]$Action = "help",
    [string]$Stage = "",
    [string]$Version = "",
    [string]$World = "keep",
    [string]$Route = "",
    [string]$ArgsText = $null,
    [string]$Out = "",
    [switch]$NoStart,
    # install: start the kiosk at the build user's logon (off by default).
    [switch]$Autostart
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version 2

$TaskName = "OCW Kiosk"
$Worlds = @("liechtenstein", "blackforest", "tenerife")

function Get-KioskDir {
    if ($env:OCW_KIOSK_DIR) { return $env:OCW_KIOSK_DIR }
    return (Join-Path $env:LOCALAPPDATA "OCW\kiosk")
}
function Get-StateDir {
    if ($env:OCW_KIOSK_STATE) { return $env:OCW_KIOSK_STATE }
    return (Join-Path $env:LOCALAPPDATA "OCW\kiosk-state")
}
function Get-EnvInt([string]$Name, [int]$Default) {
    $v = [Environment]::GetEnvironmentVariable($Name)
    if ($v -and $v -match '^\d+$') { return [int]$v }
    return $Default
}

# ---------------------------------------------------------------------------
# Pure functions (selftest covers them)
# ---------------------------------------------------------------------------

# A value for ocw-desktop.args (apps/desktop/src/argsfile.rs): single quotes
# keep backslashes literal; a value with a single quote goes in double quotes.
function ConvertTo-ArgWord([string]$Value) {
    if ($Value -notmatch "'") { return "'" + $Value + "'" }
    return '"' + ($Value -replace '\\', '\\' -replace '"', '\"') + '"'
}

# The kiosk's world: the requested one, or (keep) the one of the current
# ocw-desktop.args, else Tenerife (the VR kiosk's default).
function Resolve-KioskWorld([string]$Requested, [string]$CurrentArgs) {
    if ($Requested -and $Requested -ne "keep") {
        if ($Worlds -notcontains $Requested) { throw "world: liechtenstein, blackforest or tenerife (not '$Requested')" }
        return $Requested
    }
    foreach ($l in ($CurrentArgs -split "`n")) {
        if ($l -match '^--world .*blackforest') { return "blackforest" }
        if ($l -match '^--world .*tenerife') { return "tenerife" }
        if ($l -match '^--world .*liechtenstein') { return "liechtenstein" }
    }
    return "tenerife"
}

# The package and default route of a world, from the files in worlds\:
# @{ Package; Route; Label } or $null when the package is not there.
function Get-WorldChoice([string]$Name, [string[]]$Files) {
    switch ($Name) {
        "tenerife" {
            if ($Files -contains "tenerife.ocwpkg") { return @{ Package = "tenerife.ocwpkg"; Route = "Puerto de la Cruz - Teide"; Label = "Tenerife" } }
        }
        "blackforest" {
            if ($Files -contains "blackforest-north.ocwpkg") { return @{ Package = "blackforest-north.ocwpkg"; Route = "Baden-Baden - Mummelsee"; Label = "Black Forest north" } }
            if ($Files -contains "blackforest.ocwpkg") { return @{ Package = "blackforest.ocwpkg"; Route = "Freiburg - Schauinsland"; Label = "Black Forest south" } }
        }
        "liechtenstein" {
            if ($Files -contains "liechtenstein.ocwpkg") { return @{ Package = "liechtenstein.ocwpkg"; Route = ""; Label = "Liechtenstein" } }
        }
    }
    return $null
}

# The show mode's lines of scripts/kiosk-show.args (comments dropped); with
# a devices file among the local arguments the virtual trainer is left out
# (the real trainer rides), as on Linux.
function Get-ShowLines([string]$ShowText, [string]$LocalText) {
    $lines = @($ShowText -split "`r?`n" | Where-Object { $_.Trim() -and -not $_.StartsWith("#") })
    if ($LocalText -match '(?m)^--devices-file') {
        $lines = @($lines | Where-Object { $_ -notmatch '^--(virtual|power)' })
    }
    return $lines
}

# ocw-desktop.args of a build: the world and route, the show mode, VR and
# a 960 x 540 mirror window, then the machine's local arguments. $Override (OCW_KIOSK_ARGS)
# replaces the world and show parts.
function New-KioskArgs {
    param([string]$WorldsDir, $Choice, [string]$Route, [string]$ShowText,
          [string]$LocalText, $Override = $null)
    $lines = New-Object Collections.Generic.List[string]
    $lines.Add("# Written by ocw-kiosk.ps1 deploy (the Windows VR kiosk)")
    if ($null -ne $Override) {
        foreach ($l in ($Override -split "`r?`n")) { if ($l.Trim()) { $lines.Add($l) } }
    } else {
        if ($Choice) {
            $lines.Add("# " + $Choice.Label + " (OCW_KIOSK_WORLD)")
            $lines.Add("--world " + (ConvertTo-ArgWord (Join-Path $WorldsDir $Choice.Package)))
            $r = if ($Route) { $Route } else { $Choice.Route }
            if ($r) { $lines.Add("--route " + (ConvertTo-ArgWord $r)) }
        }
        foreach ($l in (Get-ShowLines $ShowText $LocalText)) { $lines.Add($l) }
    }
    $lines.Add("--vr")
    # A small mirror window, not the full screen (v4.71, measured on Weela:
    # the full-screen 2560 x 1600 mirror on the other GPU halved the headset's
    # rate to 36 fps; a small window keeps the 72 Hz; Q16 / Q17).
    $lines.Add("--window 960x540")
    # Never more frames than the headset shows - also while the app rides on
    # the monitor without a headset (v4.71: the fallback ran the laptop's GPU
    # at 100+ fps for the 165 Hz panel and drained its battery). In a headset
    # session xrWaitFrame paces instead.
    $lines.Add("--max-fps 72")
    foreach ($l in ($LocalText -split "`r?`n")) {
        if ($l.Trim() -and -not $l.StartsWith("#")) { $lines.Add($l) }
    }
    return ($lines -join "`n") + "`n"
}

# Whether the build in use changed (current.txt's content).
function Test-NewDeployment([string]$Running, [string]$Now) {
    if (-not $Now) { return $false }
    return ($Running.Trim() -ne $Now.Trim())
}

# The VR line of the app's log: @{ State = "on" | "off" | "unknown"; Line }.
function Get-VrState([string]$LogText) {
    $clean = $LogText -replace "\x1b\[[0-9;]*m", ""
    $off = [regex]::Match($clean, "VR is off: [^\r\n]*")
    if ($off.Success) { return @{ State = "off"; Line = $off.Value } }
    $on = [regex]::Match($clean, "(?m)^VR: [^\r\n]*")
    if ($on.Success) { return @{ State = "on"; Line = $on.Value } }
    return @{ State = "unknown"; Line = "" }
}

# The app's session changes ("VR session: idle", "... focused", "lost, ..."),
# why a session was lost ("VR session lost: ...", "VR frame: xrEndFrame
# failed ...") and the frame loop's alarms (a stall, a session never shown;
# spec 11I, v4.55), without the log's time stamps and colours, in order.
# The loop's 10 s report lines stay in app.log only.
function Get-VrSessionLines([string]$LogText) {
    $clean = $LogText -replace "\x1b\[[0-9;]*m", ""
    $pat = "VR (session: |session lost: |frame: |loop: (no frame|frames reach|the session runs))[^\r\n]*"
    return @([regex]::Matches($clean, $pat) | ForEach-Object { $_.Value })
}

# Crash pacing as on Linux: an exit within $QuickS seconds counts; three
# in a row wait 30 s, else 3 s. Returns @{ Fails; WaitS }.
function Get-RestartDelay([int]$Fails, [double]$RanS, [int]$QuickS) {
    if ($RanS -lt $QuickS) { $Fails++ } else { $Fails = 0 }
    if ($Fails -ge 3) { return @{ Fails = 0; WaitS = 30 } }
    return @{ Fails = $Fails; WaitS = 3 }
}

# The build folders to delete: all but the one in use and the newest other.
function Get-PrunableBuilds([string[]]$Names, [string]$Current, [int]$Keep = 1) {
    $others = @($Names | Where-Object { $_ -ne $Current } | Sort-Object -Descending)
    if ($others.Count -le $Keep) { return @() }
    return @($others[$Keep..($others.Count - 1)])
}

# ---------------------------------------------------------------------------
# The supervisor
# ---------------------------------------------------------------------------

function Write-KioskLog([string]$Msg) {
    $line = (Get-Date -Format "yyyy-MM-dd HH:mm:ss") + " " + $Msg
    Write-Host $line
    $state = Get-StateDir
    New-Item -ItemType Directory -Force $state | Out-Null
    Add-Content -Encoding UTF8 -Path (Join-Path $state "kiosk.log") -Value $line
}

function Read-Text([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return "" }
    # FileShare.ReadWrite: the app's log is open for writing.
    try {
        $fs = New-Object IO.FileStream($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
        try { return (New-Object IO.StreamReader($fs)).ReadToEnd() } finally { $fs.Dispose() }
    } catch { return "" }
}

# Writes a file atomically (temp file, then replace).
function Write-Atomic([string]$Path, [string]$Text) {
    $tmp = $Path + ".new"
    [IO.File]::WriteAllText($tmp, $Text)
    Move-Item -Force -LiteralPath $tmp -Destination $Path
}

function Get-FileSha([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return "" }
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLower()
}

# Ends the app: WM_CLOSE first (Bevy exits and ends the OpenXR session
# cleanly), killed after 15 s.
function Stop-App($Proc) {
    if ($null -eq $Proc -or $Proc.HasExited) { return }
    try { [void]$Proc.CloseMainWindow() } catch {}
    if (-not $Proc.WaitForExit(15000)) {
        # Still loading its world (no window yet, VR not started) or hung.
        Write-KioskLog "app did not stop, killing it"
        try { $Proc.Kill() } catch {}
        [void]$Proc.WaitForExit(5000)
    }
}

# The inner loop: exits 0 when asked to stop, 3 when a new kiosk script was
# deployed (the outer loop starts the new one).
function Invoke-Supervisor {
    $dir = Get-KioskDir
    $state = Get-StateDir
    New-Item -ItemType Directory -Force $state | Out-Null
    $poll = Get-EnvInt "OCW_KIOSK_POLL_S" 2
    $quick = Get-EnvInt "OCW_KIOSK_QUICK_S" 15
    # 60 s (v4.71; 300 before): a headset put on while the app rides on the
    # monitor showed Link's home - "the desktop mirror in full screen" - for
    # up to five minutes before the kiosk tried VR again.
    $vrRetry = Get-EnvInt "OCW_KIOSK_VR_RETRY_S" 60
    $stopFile = Join-Path $state "stop"
    $script = Join-Path $dir "ocw-kiosk.ps1"
    $me = Get-FileSha $script
    $fails = 0
    Set-Content -Path (Join-Path $state "supervisor.pid") -Value $PID
    while ($true) {
        if (Test-Path -LiteralPath $stopFile) { Remove-Item -LiteralPath $stopFile; Write-KioskLog "kiosk stopping"; return 0 }
        if (Test-Path -LiteralPath (Join-Path $dir "deploying")) { Start-Sleep -Seconds $poll; continue }
        $cur = (Read-Text (Join-Path $dir "current.txt")).Trim()
        $build = if ($cur) { Join-Path $dir $cur } else { "" }
        $exe = if ($build) { Join-Path $build "ocw-desktop.exe" } else { "" }
        if (-not $exe -or -not (Test-Path -LiteralPath $exe)) { Start-Sleep -Seconds $poll; continue }
        $applog = Join-Path $state "app.log"
        if (Test-Path -LiteralPath $applog) { Move-Item -Force -LiteralPath $applog -Destination (Join-Path $state "app.prev.log") }
        $appout = Join-Path $state "app.out.log"
        if (Test-Path -LiteralPath $appout) { Move-Item -Force -LiteralPath $appout -Destination (Join-Path $state "app.out.prev.log") }
        $ver = (Read-Text (Join-Path $build "VERSION")).Trim()
        if (-not $ver) { $ver = "unknown build" }
        Write-KioskLog "starting $ver ($cur)"
        $proc = Start-Process -FilePath $exe -WorkingDirectory $build -NoNewWindow -PassThru `
            -RedirectStandardError $applog -RedirectStandardOutput $appout
        # The handle now: without it PowerShell's process object has no
        # exit code later (the kiosk logged "code " for a crash).
        $null = $proc.Handle
        Set-Content -Path (Join-Path $state "app.pid") -Value $proc.Id
        $t0 = Get-Date
        $reason = "exited"
        $vrSeen = $false
        $retryAt = $null
        $sessionSeen = 0
        while (-not $proc.HasExited) {
            Start-Sleep -Seconds $poll
            if (Test-Path -LiteralPath $stopFile) { $reason = "stop"; break }
            if (Test-Path -LiteralPath (Join-Path $dir "deploying")) { $reason = "deploying"; Write-KioskLog "deployment in progress, stopping the app"; break }
            if (Test-NewDeployment $cur (Read-Text (Join-Path $dir "current.txt"))) { $reason = "update"; Write-KioskLog "new build deployed, restarting"; break }
            if ((Get-FileSha $script) -ne $me) { $reason = "script"; break }
            if (-not $vrSeen) {
                $vr = Get-VrState (Read-Text $applog)
                if ($vr.State -eq "on") { $vrSeen = $true; Write-KioskLog ("VR runtime: " + $vr.Line) }
                elseif ($vr.State -eq "off") {
                    $vrSeen = $true
                    if ($vrRetry -gt 0) {
                        $retryAt = (Get-Date).AddSeconds($vrRetry)
                        Write-KioskLog ($vr.Line + " - the app rides on the monitor; trying VR again in $vrRetry s")
                    } else { Write-KioskLog ($vr.Line + " - the app rides on the monitor") }
                }
            }
            $sessions = @(Get-VrSessionLines (Read-Text $applog))
            for ($i = $sessionSeen; $i -lt $sessions.Count; $i++) { Write-KioskLog $sessions[$i] }
            $sessionSeen = $sessions.Count
            if ($null -ne $retryAt -and (Get-Date) -ge $retryAt) { $reason = "vr-retry"; Write-KioskLog "restarting the app to try VR again"; break }
        }
        Stop-App $proc
        $code = if ($proc.HasExited) { "{0} (0x{0:X8})" -f $proc.ExitCode } else { "?" }
        Remove-Item -LiteralPath (Join-Path $state "app.pid") -ErrorAction SilentlyContinue
        switch ($reason) {
            "stop" { Remove-Item -LiteralPath $stopFile -ErrorAction SilentlyContinue; Write-KioskLog "kiosk stopping"; return 0 }
            "script" { Write-KioskLog "new kiosk script deployed, restarting the kiosk"; return 3 }
            "exited" {
                $d = Get-RestartDelay $fails ((Get-Date) - $t0).TotalSeconds $quick
                $fails = $d.Fails
                if ($d.WaitS -ge 30) { Write-KioskLog "app exited (code $code) 3 times in a row quickly, waiting 30 s" }
                else { Write-KioskLog "app exited (code $code), restarting in 3 s" }
                Start-Sleep -Seconds $d.WaitS
            }
            default { $fails = 0 }
        }
    }
}

# The outer loop (what the task runs): one kiosk per session (a mutex); it
# runs the supervisor of the installed script in a child PowerShell and
# starts it again when a new script was deployed (exit 3) or it failed.
function Invoke-Run {
    $created = $false
    $mutex = New-Object Threading.Mutex($true, "Local\OCW-Kiosk", [ref]$created)
    if (-not $created) { Write-Error "kiosk already running"; exit 1 }
    $dir = Get-KioskDir
    Write-KioskLog "kiosk started (dir $dir)"
    try {
        while ($true) {
            $script = Join-Path $dir "ocw-kiosk.ps1"
            if (-not (Test-Path -LiteralPath $script)) { $script = $PSCommandPath }
            $p = Start-Process -FilePath "powershell.exe" -NoNewWindow -Wait -PassThru -ArgumentList @(
                "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "`"$script`"", "supervise")
            if ($p.ExitCode -eq 0) { break }
            if ($p.ExitCode -ne 3) { Write-KioskLog "supervisor ended (code $($p.ExitCode)), starting it again in 5 s"; Start-Sleep -Seconds 5 }
        }
    } finally {
        $mutex.ReleaseMutex()
        Write-KioskLog "kiosk stopped"
    }
}

# ---------------------------------------------------------------------------
# Install, start, stop, status
# ---------------------------------------------------------------------------

function Get-StartupEntry { return (Join-Path ([Environment]::GetFolderPath("Startup")) "OCW Kiosk.cmd") }

function Test-TaskExists {
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    schtasks /query /tn $TaskName 2>$null | Out-Null
    $ok = ($LASTEXITCODE -eq 0)
    $ErrorActionPreference = $prev
    return $ok
}

# Whether the task starts at logon (its XML has a logon trigger).
function Test-TaskLogonTrigger {
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    $xml = (schtasks /query /tn $TaskName /xml 2>$null) -join "`n"
    $ErrorActionPreference = $prev
    return ($xml -match "<LogonTrigger>")
}

function Get-SupervisorAlive {
    $f = Join-Path (Get-StateDir) "supervisor.pid"
    $id = (Read-Text $f).Trim()
    if (-not $id) { return $false }
    $p = Get-Process -Id ([int]$id) -ErrorAction SilentlyContinue
    return ($null -ne $p -and $p.ProcessName -match "powershell")
}

function New-TaskXml([string]$Script, [bool]$Logon) {
    $me = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $esc = [Security.SecurityElement]::Escape($me)
    $trigger = if ($Logon) { "<Triggers><LogonTrigger><Enabled>true</Enabled><UserId>$esc</UserId></LogonTrigger></Triggers>" } else { "<Triggers />" }
    # conhost --headless: no console window on the kiosk's screen (with
    # Windows Terminal as the default console, -WindowStyle Hidden still
    # showed the kiosk's log in a terminal window, 2026-11-17).
    $arg = [Security.SecurityElement]::Escape("--headless powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$Script`" run")
    return @"
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Open Cycling World VR kiosk (scripts/ocw-kiosk.ps1)</Description></RegistrationInfo>
  $trigger
  <Principals><Principal id="Author"><UserId>$esc</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <Enabled>true</Enabled>
  </Settings>
  <Actions Context="Author"><Exec><Command>conhost.exe</Command><Arguments>$arg</Arguments></Exec></Actions>
</Task>
"@
}

function Invoke-Install {
    $dir = Get-KioskDir
    New-Item -ItemType Directory -Force $dir | Out-Null
    $script = Join-Path $dir "ocw-kiosk.ps1"
    if ($PSCommandPath -ne $script) { Copy-Item -Force -LiteralPath $PSCommandPath -Destination $script }
    $xml = Join-Path $env:TEMP "ocw-kiosk-task.xml"
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    $how = ""
    # With -Autostart: a logon trigger, else (refused) a task without one
    # plus the Startup folder entry; without: a task without a trigger.
    $tries = if ($Autostart) { @($true, $false) } else { @($false) }
    foreach ($logon in $tries) {
        [IO.File]::WriteAllText($xml, (New-TaskXml $script $logon), [Text.Encoding]::Unicode)
        schtasks /create /tn $TaskName /xml $xml /f 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { $how = if ($logon) { "logon" } else { "startup" }; break }
    }
    $ErrorActionPreference = $prev
    Remove-Item -LiteralPath $xml -ErrorAction SilentlyContinue
    if (-not $how) { throw "schtasks /create failed: cannot register the task '$TaskName'" }
    $startup = Get-StartupEntry
    if (-not $Autostart) {
        Remove-Item -LiteralPath $startup -ErrorAction SilentlyContinue
        Write-KioskLog "installed: task '$TaskName' without autostart (start it by hand: ocw-kiosk.ps1 start, or the desktop shortcut)"
    } elseif ($how -eq "startup") {
        # The logon trigger was refused: the Startup folder starts the task.
        Set-Content -Encoding ascii -Path $startup -Value "@schtasks /run /tn `"$TaskName`" >nul"
        Write-KioskLog "autostart installed: Startup folder entry, task '$TaskName' (no logon trigger allowed)"
    } else {
        Remove-Item -LiteralPath $startup -ErrorAction SilentlyContinue
        Write-KioskLog "autostart installed: task '$TaskName' at logon"
    }
    if (-not $NoStart) { Invoke-Start }
}

function Invoke-Start {
    if (Get-SupervisorAlive) { Write-Host "kiosk already running"; return }
    if (-not (Test-TaskExists)) { throw "the kiosk is not installed (ocw-kiosk.ps1 install)" }
    Remove-Item -LiteralPath (Join-Path (Get-StateDir) "stop") -ErrorAction SilentlyContinue
    schtasks /run /tn $TaskName | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "schtasks /run failed ($LASTEXITCODE): is the build user logged in on the desktop?" }
    Write-Host "started; log: $(Join-Path (Get-StateDir) 'kiosk.log')"
}

function Invoke-Stop {
    $state = Get-StateDir
    New-Item -ItemType Directory -Force $state | Out-Null
    if (Get-SupervisorAlive) {
        Set-Content -Path (Join-Path $state "stop") -Value "stop"
        for ($i = 0; $i -lt 40 -and (Get-SupervisorAlive); $i++) { Start-Sleep -Milliseconds 500 }
    }
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    if (Test-TaskExists) { schtasks /end /tn $TaskName 2>&1 | Out-Null }
    $ErrorActionPreference = $prev
    $app = (Read-Text (Join-Path $state "app.pid")).Trim()
    if ($app) { Stop-Process -Id ([int]$app) -Force -ErrorAction SilentlyContinue }
    Remove-Item -LiteralPath (Join-Path $state "stop") -ErrorAction SilentlyContinue
    Write-Host "stopped"
}

function Invoke-Uninstall {
    Invoke-Stop
    $prev = $ErrorActionPreference; $ErrorActionPreference = "Continue"
    if (Test-TaskExists) { schtasks /delete /tn $TaskName /f 2>&1 | Out-Null }
    $ErrorActionPreference = $prev
    Remove-Item -LiteralPath (Get-StartupEntry) -ErrorAction SilentlyContinue
    Write-Host "autostart removed"
}

function Invoke-Status {
    $dir = Get-KioskDir; $state = Get-StateDir
    $cur = (Read-Text (Join-Path $dir "current.txt")).Trim()
    $ver = if ($cur) { (Read-Text (Join-Path (Join-Path $dir $cur) "VERSION")).Trim() } else { "" }
    Write-Host ("kiosk folder: " + $dir)
    Write-Host ("build: " + $(if ($ver) { "$ver ($cur)" } else { "none deployed" }))
    $auto = if (Test-TaskExists) { "task '$TaskName'" + $(if (Test-Path -LiteralPath (Get-StartupEntry)) { " (started from the Startup folder)" } elseif (Test-TaskLogonTrigger) { " (at logon)" } else { ", no autostart (started by hand)" }) } else { "not installed" }
    Write-Host ("autostart: " + $auto)
    Write-Host ("kiosk: " + $(if (Get-SupervisorAlive) { "running" } else { "not running" }))
    $app = (Read-Text (Join-Path $state "app.pid")).Trim()
    $p = if ($app) { Get-Process -Id ([int]$app) -ErrorAction SilentlyContinue } else { $null }
    Write-Host ("app: " + $(if ($p) { "running (pid $app, since $($p.StartTime))" } else { "not running" }))
    $vr = Get-VrState (Read-Text (Join-Path $state "app.log"))
    Write-Host ("vr: " + $(if ($vr.Line) { $vr.Line } else { "no VR line in app.log yet" }))
    $sess = @(Get-VrSessionLines (Read-Text (Join-Path $state "app.log")))
    $sessText = if ($sess.Count -gt 0) { $sess[$sess.Count - 1] } elseif ($vr.State -eq "on") { "VR session: idle (no state change yet: the runtime waits for the headset - worn, awake, Link on)" } else { "" }
    if ($sessText) { Write-Host ("session: " + $sessText) }
    $log = Join-Path $state "kiosk.log"
    if (Test-Path -LiteralPath $log) { Get-Content -LiteralPath $log -Tail 12 }
}

# A picture of the primary monitor, taken in the desktop session (a one-off
# task like scripts/win-gui-run.ps1; SSH sessions see no screen).
function Invoke-Screenshot {
    $state = Get-StateDir
    New-Item -ItemType Directory -Force $state | Out-Null
    $outPath = if ($Out) { [IO.Path]::GetFullPath($Out) } else { Join-Path $state ("screen-" + (Get-Date -Format "yyyyMMdd-HHmmss") + ".png") }
    New-Item -ItemType Directory -Force (Split-Path $outPath) | Out-Null
    $name = "ocw-kiosk-shot-$PID"
    $ps1 = Join-Path $env:TEMP "$name.ps1"
    @"
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
Add-Type -Namespace W -Name D -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[void][W.D]::SetProcessDPIAware()
`$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
`$bmp = New-Object System.Drawing.Bitmap `$b.Width, `$b.Height
`$g = [System.Drawing.Graphics]::FromImage(`$bmp)
`$g.CopyFromScreen(`$b.Location, [System.Drawing.Point]::Empty, `$b.Size)
`$bmp.Save('$($outPath -replace "'", "''")', [System.Drawing.Imaging.ImageFormat]::Png)
"@ | Set-Content -Encoding ascii $ps1
    Remove-Item -LiteralPath $outPath -ErrorAction SilentlyContinue
    schtasks /create /tn $name /tr "powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File `"$ps1`"" /sc once /st 00:00 /sd 01/01/2099 /it /f | Out-Null
    try {
        schtasks /run /tn $name | Out-Null
        for ($i = 0; $i -lt 60 -and -not (Test-Path -LiteralPath $outPath); $i++) { Start-Sleep -Milliseconds 500 }
        if (-not (Test-Path -LiteralPath $outPath)) { throw "no screenshot after 30 s: is the build user logged in on the desktop?" }
        Start-Sleep -Milliseconds 500
        Write-Host "screenshot: $outPath"
    } finally {
        schtasks /delete /tn $name /f 2>$null | Out-Null
        Remove-Item -LiteralPath $ps1 -ErrorAction SilentlyContinue
    }
}

# ---------------------------------------------------------------------------
# Deploy (called by scripts/deploy-test.sh on the machine)
# ---------------------------------------------------------------------------

# $Stage holds ocw-desktop.exe, openxr_loader.dll (+ .LICENSE.txt),
# materials\, ocw-kiosk.ps1, kiosk-show.args and optionally worlds\ (world
# packages, manifests, routes whose content changed).
function Invoke-Deploy {
    if (-not $Stage -or -not (Test-Path -LiteralPath (Join-Path $Stage "ocw-desktop.exe"))) { throw "deploy: -Stage DIR with ocw-desktop.exe" }
    # Absolute: [IO.File] resolves against the process's folder, not PowerShell's.
    $Stage = (Resolve-Path -LiteralPath $Stage).Path
    $dir = Get-KioskDir
    $worldsDir = Join-Path $dir "worlds"
    $builds = Join-Path $dir "builds"
    New-Item -ItemType Directory -Force $dir, $worldsDir, $builds | Out-Null
    # World files: copied while the app is stopped (it holds its package open).
    $newWorld = @(Get-ChildItem -LiteralPath (Join-Path $Stage "worlds") -File -ErrorAction SilentlyContinue)
    if ($newWorld.Count -gt 0) {
        $flag = Join-Path $dir "deploying"
        Set-Content -Path $flag -Value (Get-Date -Format o)
        try {
            $app = (Read-Text (Join-Path (Get-StateDir) "app.pid")).Trim()
            if ($app -and (Get-SupervisorAlive)) {
                for ($i = 0; $i -lt 60 -and (Get-Process -Id ([int]$app) -ErrorAction SilentlyContinue); $i++) { Start-Sleep -Milliseconds 500 }
            }
            foreach ($f in $newWorld) {
                $dst = Join-Path $worldsDir $f.Name
                Copy-Item -Force -LiteralPath $f.FullName -Destination ($dst + ".new")
                Move-Item -Force -LiteralPath ($dst + ".new") -Destination $dst
                Write-Host ("world file: " + $f.Name + " " + (Get-FileSha $dst))
            }
        } finally { Remove-Item -LiteralPath $flag -ErrorAction SilentlyContinue }
    }
    # The new build folder.
    $id = Get-Date -Format "yyyyMMdd-HHmmss"
    $b = Join-Path $builds $id
    New-Item -ItemType Directory -Force $b | Out-Null
    foreach ($f in @("ocw-desktop.exe", "openxr_loader.dll", "openxr_loader.LICENSE.txt")) {
        $src = Join-Path $Stage $f
        if (Test-Path -LiteralPath $src) { Copy-Item -LiteralPath $src -Destination $b }
        elseif ($f -ne "ocw-desktop.exe") { Write-Warning "$f missing: VR cannot start (the app runs without it)" }
    }
    if (Test-Path -LiteralPath (Join-Path $Stage "materials")) { Copy-Item -Recurse -LiteralPath (Join-Path $Stage "materials") -Destination (Join-Path $b "materials") }
    else { Write-Warning "no material library: the app draws flat colours" }
    $cur = (Read-Text (Join-Path $dir "current.txt")).Trim()
    $curArgs = if ($cur) { Read-Text (Join-Path (Join-Path $dir $cur) "ocw-desktop.args") } else { "" }
    $w = Resolve-KioskWorld $World $curArgs
    $files = @(Get-ChildItem -LiteralPath $worldsDir -File | ForEach-Object { $_.Name })
    $choice = Get-WorldChoice $w $files
    if (-not $choice) { Write-Warning "world '$w' is not installed in ${worldsDir}: the app rides its built-in route" }
    $local = Read-Text (Join-Path $dir "kiosk-local.args")
    $show = Read-Text (Join-Path $Stage "kiosk-show.args")
    $override = if ($PSBoundParameters.ContainsKey("ArgsText") -or $null -ne $env:OCW_KIOSK_ARGS) { if ($null -ne $ArgsText) { $ArgsText } else { $env:OCW_KIOSK_ARGS } } else { $null }
    $argsText = New-KioskArgs -WorldsDir $worldsDir -Choice $choice -Route $Route -ShowText $show -LocalText $local -Override $override
    [IO.File]::WriteAllText((Join-Path $b "ocw-desktop.args"), $argsText)
    [IO.File]::WriteAllText((Join-Path $b "VERSION"), $Version + "`n")
    Write-Host "default arguments ($b\ocw-desktop.args):"
    $argsText -split "`n" | Where-Object { $_ -and -not $_.StartsWith("#") } | ForEach-Object { Write-Host "  $_" }
    # The kiosk script (the running kiosk restarts on a changed one), then
    # the switch to the new build - last.
    $ks = Join-Path $Stage "ocw-kiosk.ps1"
    if (Test-Path -LiteralPath $ks) {
        $dst = Join-Path $dir "ocw-kiosk.ps1"
        if ((Get-FileSha $ks) -ne (Get-FileSha $dst)) { Copy-Item -Force -LiteralPath $ks -Destination ($dst + ".new"); Move-Item -Force -LiteralPath ($dst + ".new") -Destination $dst }
    }
    Write-Atomic (Join-Path $dir "current.txt") ("builds\" + $id + "`n")
    Write-Host "deployed $Version to $b"
    # Old builds (a running exe cannot be deleted: skipped, next time).
    foreach ($old in (Get-PrunableBuilds @(Get-ChildItem -LiteralPath $builds -Directory | ForEach-Object { $_.Name }) $id)) {
        Remove-Item -Recurse -Force -LiteralPath (Join-Path $builds $old) -ErrorAction SilentlyContinue
    }
    if ((Test-TaskExists) -and -not (Get-SupervisorAlive)) {
        # A stopped kiosk stays stopped (2026-11-18: a deploy started it on
        # its own); a running one restarts the app on the new build.
        Write-Host "the kiosk is installed but not running: left stopped (start it: $dir\ocw-kiosk.ps1 start, or the desktop shortcut)"
    } elseif (-not (Test-TaskExists)) {
        Write-Host "the kiosk is not installed: $dir\ocw-kiosk.ps1 install (as the build user)"
    }
}

# For scripts/deploy-test.sh: the kiosk's world after this deploy
# ("@@world NAME") and the SHA-256 of every installed world file
# ("@@sha HASH NAME"), so only changed files are sent.
function Invoke-WorldInfo {
    $dir = Get-KioskDir
    $cur = (Read-Text (Join-Path $dir "current.txt")).Trim()
    $curArgs = if ($cur) { Read-Text (Join-Path (Join-Path $dir $cur) "ocw-desktop.args") } else { "" }
    Write-Host ("@@world " + (Resolve-KioskWorld $World $curArgs))
    Write-Host ("@@dir " + $dir)
    foreach ($f in @(Get-ChildItem -LiteralPath (Join-Path $dir "worlds") -File -ErrorAction SilentlyContinue)) {
        if ($f.Name -like "*.new") { continue }
        Write-Host ("@@sha " + (Get-FileSha $f.FullName) + " " + $f.Name)
    }
}

# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

function Invoke-SelfTest {
    $script:fails = 0; $script:n = 0
    function Check([string]$What, $Got, $Want) {
        $script:n++
        if ("$Got" -ne "$Want") { $script:fails++; Write-Host "FAIL: $What`n  got:  $Got`n  want: $Want" }
    }
    Check "quote plain" (ConvertTo-ArgWord 'C:\a b\t.ocwpkg') "'C:\a b\t.ocwpkg'"
    Check "quote with '" (ConvertTo-ArgWord "it's \x") '"it''s \\x"'.Replace("''", "'")
    Check "world keep, none" (Resolve-KioskWorld "keep" "") "tenerife"
    Check "world keep, black forest" (Resolve-KioskWorld "keep" "# x`n--world 'C:\k\worlds\blackforest-north.ocwpkg'`n") "blackforest"
    Check "world keep, liechtenstein" (Resolve-KioskWorld "keep" "--world 'C:\w\liechtenstein.ocwpkg'") "liechtenstein"
    Check "world asked" (Resolve-KioskWorld "blackforest" "--world 'x\tenerife.ocwpkg'") "blackforest"
    $bad = $false; try { Resolve-KioskWorld "mars" "" | Out-Null } catch { $bad = $true }
    Check "world unknown refused" $bad $true
    Check "tenerife route" (Get-WorldChoice "tenerife" @("a.route", "tenerife.ocwpkg")).Route "Puerto de la Cruz - Teide"
    Check "black forest north first" (Get-WorldChoice "blackforest" @("blackforest.ocwpkg", "blackforest-north.ocwpkg")).Package "blackforest-north.ocwpkg"
    Check "black forest south" (Get-WorldChoice "blackforest" @("blackforest.ocwpkg")).Route "Freiburg - Schauinsland"
    Check "world missing" ($null -eq (Get-WorldChoice "tenerife" @("liechtenstein.ocwpkg"))) $true
    $show = "# show`n--virtual`n--power 500`n--cmd `"ride.collisions off`"`n--no-hearts`n"
    Check "show lines" ((Get-ShowLines $show "") -join "|") '--virtual|--power 500|--cmd "ride.collisions off"|--no-hearts'
    Check "show lines, real trainer" ((Get-ShowLines $show "--devices-file x") -join "|") '--cmd "ride.collisions off"|--no-hearts'
    $a = New-KioskArgs -WorldsDir 'C:\k\worlds' -Choice (Get-WorldChoice "tenerife" @("tenerife.ocwpkg")) -Route "" -ShowText $show -LocalText "# local`n--monitor 1`n"
    $want = "# Written by ocw-kiosk.ps1 deploy (the Windows VR kiosk)`n# Tenerife (OCW_KIOSK_WORLD)`n--world 'C:\k\worlds\tenerife.ocwpkg'`n--route 'Puerto de la Cruz - Teide'`n--virtual`n--power 500`n--cmd `"ride.collisions off`"`n--no-hearts`n--vr`n--window 960x540`n--max-fps 72`n--monitor 1`n"
    Check "kiosk args" $a $want
    $a = New-KioskArgs -WorldsDir 'C:\k' -Choice (Get-WorldChoice "tenerife" @("tenerife.ocwpkg")) -Route "Los Cristianos - Teide" -ShowText "" -LocalText ""
    Check "kiosk args, route" (($a -split "`n") -match '^--route') "--route 'Los Cristianos - Teide'"
    $a = New-KioskArgs -WorldsDir 'C:\k' -Choice $null -Route "" -ShowText $show -LocalText "" -Override "--test-route hills"
    Check "kiosk args, override" (($a -split "`n" | Where-Object { $_ -and -not $_.StartsWith("#") }) -join "|") "--test-route hills|--vr|--window 960x540|--max-fps 72"
    $a = New-KioskArgs -WorldsDir 'C:\k' -Choice $null -Route "" -ShowText $show -LocalText ""
    Check "kiosk args, no world" (($a -split "`n") -match '^--world').Count 0
    Check "deployment same" (Test-NewDeployment "builds\1" "builds\1`r`n") $false
    Check "deployment new" (Test-NewDeployment "builds\1" "builds\2`n") $true
    Check "deployment gone" (Test-NewDeployment "builds\1" "") $false
    Check "vr on" (Get-VrState "x`n`e[32mINFO`e[0m`nVR: Oculus 1.2 on Oculus Quest (khr), Rgba8UnormSrgb, 72 Hz`n").State "on"
    Check "vr on line" (Get-VrState "VR: Oculus on Quest (x)`r`nmore").Line "VR: Oculus on Quest (x)"
    Check "vr off" (Get-VrState "`e[33mWARN`e[0m ocw_desktop::vr: VR is off: no OpenXR runtime`n").Line "VR is off: no OpenXR runtime"
    # No autostart by default (2026-11-18): a task without a trigger; the
    # logon trigger only with -Autostart.
    Check "task without autostart" ((New-TaskXml "C:\k\ocw-kiosk.ps1" $false) -match "<LogonTrigger>") $false
    Check "task with -Autostart" ((New-TaskXml "C:\k\ocw-kiosk.ps1" $true) -match "<LogonTrigger>") $true
    Check "session lines" ((Get-VrSessionLines "a`n2026 INFO x: VR session: ready`r`nb`nVR session: focused`n") -join "|") "VR session: ready|VR session: focused"
    Check "loss lines" ((Get-VrSessionLines "x VR session lost: the VR session was lost`nVR loop 10 s: 72.0 waits/s`nVR frame: xrEndFrame failed after 1.0 ms: lost`nVR loop: xrWaitFrame took 300 ms`nVR loop: no frame reached the runtime for 600 ms`n") -join "|") "VR session lost: the VR session was lost|VR frame: xrEndFrame failed after 1.0 ms: lost|VR loop: no frame reached the runtime for 600 ms"
    Check "no session lines" (@(Get-VrSessionLines "nothing")).Count 0
    Check "vr unknown" (Get-VrState "starting").State "unknown"
    $d = Get-RestartDelay 0 100 15; Check "slow exit" "$($d.Fails) $($d.WaitS)" "0 3"
    $d = Get-RestartDelay 1 2 15; Check "second quick exit" "$($d.Fails) $($d.WaitS)" "2 3"
    $d = Get-RestartDelay 2 2 15; Check "third quick exit" "$($d.Fails) $($d.WaitS)" "0 30"
    Check "prune" ((Get-PrunableBuilds @("20261101-1", "20261102-1", "20261103-1", "20261104-1") "20261104-1") -join ",") "20261102-1,20261101-1"
    Check "prune keeps one other" (@(Get-PrunableBuilds @("20261101-1", "20261102-1") "20261102-1")).Count 0
    # Deployment end to end in a temp folder (no app started).
    $tmp = Join-Path $env:TEMP ("ocw-kiosk-selftest-" + $PID)
    try {
        $env:OCW_KIOSK_DIR = Join-Path $tmp "kiosk"; $env:OCW_KIOSK_STATE = Join-Path $tmp "state"
        $st = Join-Path $tmp "stage"
        New-Item -ItemType Directory -Force (Join-Path $st "worlds"), (Join-Path $st "materials") | Out-Null
        Set-Content (Join-Path $st "ocw-desktop.exe") "fake"
        Set-Content (Join-Path $st "openxr_loader.dll") "fake"
        Set-Content (Join-Path $st "materials\manifest.toml") "x"
        Set-Content (Join-Path $st "worlds\tenerife.ocwpkg") "pkg"
        Set-Content (Join-Path $st "kiosk-show.args") $show
        $script:Stage = $st; $script:Version = "abc1234 2026-11-17"; $script:World = "keep"
        Invoke-Deploy *> $null
        $k = $env:OCW_KIOSK_DIR
        $cur = (Read-Text (Join-Path $k "current.txt")).Trim()
        Check "deploy current.txt" ($cur -like "builds\2*") $true
        Check "deploy args world" ((Read-Text (Join-Path (Join-Path $k $cur) "ocw-desktop.args")) -match [regex]::Escape("--world '" + (Join-Path $k "worlds\tenerife.ocwpkg") + "'")) $true
        Check "deploy materials" (Test-Path (Join-Path (Join-Path $k $cur) "materials\manifest.toml")) $true
        Check "deploy version" (Read-Text (Join-Path (Join-Path $k $cur) "VERSION")).Trim() "abc1234 2026-11-17"
        Check "deploy no flag left" (Test-Path (Join-Path $k "deploying")) $false
        # A second deploy keeps the world (blackforest asked, not installed -> keep reading args).
        Start-Sleep -Seconds 1
        Remove-Item -Recurse (Join-Path $st "worlds")
        Invoke-Deploy *> $null
        $cur2 = (Read-Text (Join-Path $k "current.txt")).Trim()
        Check "second deploy is new" (Test-NewDeployment $cur $cur2) $true
        Check "second deploy keeps the world" ((Read-Text (Join-Path (Join-Path $k $cur2) "ocw-desktop.args")) -match "tenerife.ocwpkg") $true
    } finally {
        Remove-Item Env:\OCW_KIOSK_DIR, Env:\OCW_KIOSK_STATE -ErrorAction SilentlyContinue
        Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
    }
    Write-Host "selftest: $($script:n - $script:fails) of $($script:n) passed"
    if ($script:fails -gt 0) { exit 1 }
}

switch ($Action) {
    "run" { Invoke-Run }
    "supervise" { exit (Invoke-Supervisor) }
    "install" { Invoke-Install }
    "uninstall" { Invoke-Uninstall }
    "start" { Invoke-Start }
    "stop" { Invoke-Stop }
    "restart" { Invoke-Stop; Invoke-Start }
    "status" { Invoke-Status }
    "screenshot" { Invoke-Screenshot }
    "deploy" { Invoke-Deploy }
    "worldinfo" { Invoke-WorldInfo }
    "selftest" { Invoke-SelfTest }
    default { Get-Help $PSCommandPath -Detailed | Out-String | Write-Host; exit 2 }
}
# Done: not the exit code of the last native command (schtasks /query).
exit 0
