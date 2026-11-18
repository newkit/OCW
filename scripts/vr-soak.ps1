<#
.SYNOPSIS
  A long VR run on a Windows machine with a headset (spec 14 Phase 11, 11I):
  the show ride in the headset for -Minutes, `vr status` (session, frame
  times, missed frames) logged every -StatusEveryS seconds, a left-eye
  screenshot every -ShotEveryS seconds, then `quit` - a clean exit is part
  of the test. Runs in the build user's desktop session through
  scripts/win-gui-run.ps1 (over `scripts/remote.sh MACHINE run`).

    scripts\vr-soak.ps1 -Minutes 30 -World D:\w\tenerife.ocwpkg -Route "Puerto de la Cruz - Teide"

  Output in -Out (default test-output\vr-soak-TIME): app.log (RUST_BACKTRACE=1),
  the arguments file, eye-NNN.jpg (the left eye, every -ShotEveryS seconds),
  mirror-video.mp4 (the mirror window, -VideoS seconds from -VideoAtS, by the
  app's own `video` recorder; needs ffmpeg on PATH - without it the summary
  says so), media.txt (one line per picture / video: what it shows, for
  test-output\index.md on the coordinator), summary.txt (exit code, session
  changes, the last frame statistics). Every test run records pictures and,
  where it moves, a video (CLAUDE.md). Exit code: the app's.
#>
param(
    [int]$Minutes = 30,
    [string]$Exe = "target\release\ocw-desktop.exe",
    [string]$World = "",
    [string]$Route = "",
    [int]$StatusEveryS = 30,
    [int]$ShotEveryS = 60,
    # The mirror window as a video: this many seconds (0: none) …
    [int]$VideoS = 30,
    # … starting this many seconds after the app started.
    [int]$VideoAtS = 90,
    [string]$Out = "",
    [string[]]$Extra = @(),
    # A small window instead of the fullscreen mirror (measures the mirror's cost).
    [switch]$Windowed,
    # No app, no headset: only check that the video step works in the desktop
    # session (ffmpeg found, a 5 s capture of the desktop through the same
    # scheduled-task path) -> desktop-5s.mp4 in -Out.
    [switch]$CheckVideo,
    # The mirror window's size with -Windowed (v4.71: the kiosk's is 960x540).
    [string]$Window = "640x360"
)
$ErrorActionPreference = "Stop"

# ffmpeg for the mirror video (the app's `video` recorder looks only on PATH):
# PATH, else winget's links (user or machine scope), else its package folder.
function Find-Ffmpeg {
    $c = Get-Command ffmpeg -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    $cands = @(
        (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links\ffmpeg.exe"),
        (Join-Path $env:ProgramFiles "WinGet\Links\ffmpeg.exe")
    )
    foreach ($p in $cands) { if (Test-Path $p) { return $p } }
    foreach ($root in @((Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages"), (Join-Path $env:ProgramFiles "WinGet\Packages"))) {
        $f = Get-ChildItem $root -Filter "ffmpeg.exe" -Recurse -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($f) { return $f.FullName }
    }
    return $null
}
$ffmpeg = Find-Ffmpeg
if (-not $Out) { $Out = "test-output\vr-soak-" + (Get-Date -Format "yyyyMMdd-HHmmss") }
New-Item -ItemType Directory -Force $Out | Out-Null
$outAbs = (Resolve-Path $Out).Path
if ($CheckVideo) {
    if (-not $ffmpeg) { Write-Host "video check: ffmpeg not found (PATH, WinGet Links / Packages)"; exit 2 }
    $mp4 = Join-Path $outAbs "desktop-5s.mp4"
    Remove-Item $mp4 -ErrorAction SilentlyContinue
    & (Join-Path $PSScriptRoot "win-gui-run.ps1") -Exe $ffmpeg -Log (Join-Path $outAbs "ffmpeg.log") -TimeoutSec 60 `
        -Arguments "-hide_banner -y -f gdigrab -framerate 15 -t 5 -i desktop -vf scale=960:-2 -c:v libx264 -pix_fmt yuv420p `"$mp4`"" | Out-Null
    $ok = (Test-Path $mp4) -and (Get-Item $mp4).Length -gt 1000
    [IO.File]::WriteAllLines((Join-Path $outAbs "media.txt"), @("desktop-5s.mp4`tthe build user's desktop, 5 s (video check of the VR runs' recording path, no app)"))
    Write-Host ("video check: ffmpeg $ffmpeg -> " + $(if ($ok) { "desktop-5s.mp4 $([math]::Round((Get-Item $mp4).Length / 1KB)) KB" } else { "no video, see ffmpeg.log" }))
    exit $(if ($ok) { 0 } else { 1 })
}
$secs = $Minutes * 60
$lines = New-Object Collections.Generic.List[string]
if ($World) { $lines.Add("--world '$World'") }
if ($Route) { $lines.Add("--route '$Route'") }
foreach ($l in @("--virtual", "--power 500", '--cmd "ride.collisions off"', "--no-hearts", '--cmd "camera.shake off"', "--vr")) { $lines.Add($l) }
$lines.Add($(if ($Windowed) { "--window $Window" } else { "--fullscreen" }))
for ($t = 20; $t -lt $secs; $t += $StatusEveryS) { $lines.Add("--at `"$t=vr status`"") }
$n = 0
for ($t = 60; $t -lt $secs; $t += $ShotEveryS) {
    $n++
    $lines.Add("--at `"$t=screenshot " + (Join-Path $outAbs ("eye-{0:D3}.jpg" -f $n)) + "`"")
}
if ($VideoS -gt 0 -and ($VideoAtS + $VideoS) -lt $secs) {
    $lines.Add("--at `"$VideoAtS=video start`"")
    $lines.Add("--at `"$($VideoAtS + $VideoS)=video stop`"")
}
$lines.Add("--at `"$secs=quit`"")
foreach ($e in $Extra) { $lines.Add($e) }
$argsFile = Join-Path $outAbs "ocw-desktop.args"
[IO.File]::WriteAllText($argsFile, ($lines -join "`n") + "`n")
$t0 = Get-Date
$envVars = @("RUST_BACKTRACE=1", "OCW_ARGS_FILE=$argsFile")
# The app finds ffmpeg on its PATH: put ffmpeg's folder first.
# (win-gui-run sets it in a .cmd: %PATH% is the desktop session's own.)
if ($ffmpeg) { $envVars += "PATH=$(Split-Path $ffmpeg);%PATH%" }
& (Join-Path $PSScriptRoot "win-gui-run.ps1") -Exe $Exe -Log (Join-Path $outAbs "app.log") `
    -TimeoutSec ($secs + 180) -EnvVars $envVars | Out-Null
$code = $LASTEXITCODE
$ran = [math]::Round(((Get-Date) - $t0).TotalSeconds)
$log = if (Test-Path (Join-Path $outAbs "app.log")) { Get-Content (Join-Path $outAbs "app.log") | ForEach-Object { $_ -replace "\x1b\[[0-9;]*m", "" } } else { @() }
$sum = New-Object Collections.Generic.List[string]
$sum.Add("exit code: $code (0x{0:X8}) after $ran s of $secs s planned" -f $code)
$sum.Add("clean exit: " + [bool]($log -match "session torn down"))
$sum.Add("panics: " + @($log -match "panicked").Count)
$sum.Add("session changes:")
$log | Select-String -Pattern "VR session: |^VR: |VR is off" | ForEach-Object { $sum.Add("  " + $_.Line) }
$sum.Add("session losses: " + @($log -match "VR session lost: ").Count + ", reconnects done: " + @($log -match "VR: new session created").Count)
$sum.Add("failed frame calls: " + @($log -match "VR frame: ").Count + ", stalls: " + @($log -match "VR loop: no frame reached").Count + ", sessions never shown: " + @($log -match "does not show it").Count)
$log | Select-String -Pattern "VR session lost: |VR frame: |VR loop: (no frame|the session runs)" | Select-Object -First 20 | ForEach-Object { $sum.Add("  " + $_.Line) }
$sum.Add("frame loop (the last reports, every 10 s):")
$log | Select-String -Pattern "VR loop \d+ s:" | Select-Object -Last 4 | ForEach-Object { $sum.Add("  " + $_.Line) }
$sum.Add("frame statistics (vr status):")
$log | Select-String -Pattern "frames? .*missed|missed" | Select-Object -Last 12 | ForEach-Object { $sum.Add("  " + $_.Line) }
$sum.Add("render distance steps: " + @($log -match "view distance:").Count)
$sum.Add("screenshots: " + @(Get-ChildItem $outAbs -Filter "eye-*.jpg").Count)
# The app's video (written to Videos\OCW): copied beside the run's pictures.
$media = New-Object Collections.Generic.List[string]
$vdir = Join-Path ([Environment]::GetFolderPath("MyVideos")) "OCW"
$video = Get-ChildItem $vdir -Filter "*.mp4" -ErrorAction SilentlyContinue | Where-Object { $_.LastWriteTime -gt $t0 } | Sort-Object LastWriteTime | Select-Object -Last 1
if ($video) {
    Copy-Item $video.FullName (Join-Path $outAbs "mirror-video.mp4") -Force
    $sum.Add("video: mirror-video.mp4 ($([math]::Round($video.Length / 1MB, 1)) MB)")
    $media.Add("mirror-video.mp4`tthe monitor mirror (left eye) during the VR run, $VideoS s from ${VideoAtS} s")
} elseif ($VideoS -gt 0) {
    $why = if (-not $ffmpeg) { "ffmpeg is not installed (PATH, WinGet Links / Packages)" } else { "see app.log ('video'; ffmpeg: $ffmpeg)" }
    $sum.Add("video: none - $why")
}
Get-ChildItem $outAbs -Filter "*.jpg" | Sort-Object Name | ForEach-Object { $media.Add("$($_.Name)`tthe left eye in the headset (VR run, $($_.LastWriteTime.ToString('HH:mm:ss')))") }
[IO.File]::WriteAllLines((Join-Path $outAbs "media.txt"), $media)
$sum.Add("last lines:")
$log | Select-Object -Last 15 | ForEach-Object { $sum.Add("  " + $_) }
[IO.File]::WriteAllLines((Join-Path $outAbs "summary.txt"), $sum)
$sum | ForEach-Object { Write-Host $_ }
exit $code
