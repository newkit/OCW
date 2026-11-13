# Moves the Rust toolchain and the compiler cache of a Windows build machine
# off C: (spec §11.2, v2.63): CARGO_HOME (%USERPROFILE%\.cargo), RUSTUP_HOME
# (%USERPROFILE%\.rustup) and the sccache cache (%LOCALAPPDATA%\Mozilla\sccache\cache)
# go to the build drive (default B:\toolchain\cargo, \rustup, \sccache), so
# the system drive does not fill up with toolchains and cache.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts/windows-move-toolchain.ps1
#       dry run (default): what would be moved, the sizes on C:, free space
#   ... -Apply                 copy, set the user environment variables
#                              (CARGO_HOME, RUSTUP_HOME, SCCACHE_DIR, PATH),
#                              verify `cargo --version` / `rustc --version`
#                              from the new place, then remove the old dirs
#   ... -Apply -KeepOld        the same, but keep the old dirs
#   ... -Target D:\rust        another destination
#
# From the coordinator: make windows-move-toolchain MACHINE=Win11Builder
# [APPLY=1]. Safe order: nothing is removed before the copy is complete
# (same file count and bytes) and the toolchain runs from the new place;
# a failed check restores the old environment variables. Not while cargo,
# rustc, a linker or sccache run (-Force overrides). New SSH sessions get the
# new variables (scripts/remote.sh puts %CARGO_HOME%\bin first on PATH).
param(
    [string]$Target = 'B:\toolchain',
    [switch]$Apply,
    [switch]$KeepOld,
    [switch]$Force
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Size($p) {
    if (-not (Test-Path -LiteralPath $p)) { return @{ files = 0; bytes = [int64]0 } }
    $m = Get-ChildItem -LiteralPath $p -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum
    return @{ files = [int64]$m.Count; bytes = [int64]$m.Sum }
}
function Gb($b) { '{0:N1} GB' -f ($b / 1GB) }
function UserEnv($n) { [Environment]::GetEnvironmentVariable($n, 'User') }

$cargoOld = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
$rustupOld = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE '.rustup' }
$sccacheOld = if ($env:SCCACHE_DIR) { $env:SCCACHE_DIR } else { Join-Path $env:LOCALAPPDATA 'Mozilla\sccache\cache' }
$moves = @(
    @{ name = 'CARGO_HOME'; from = $cargoOld; to = (Join-Path $Target 'cargo') },
    @{ name = 'RUSTUP_HOME'; from = $rustupOld; to = (Join-Path $Target 'rustup') },
    @{ name = 'SCCACHE_DIR'; from = $sccacheOld; to = (Join-Path $Target 'sccache') }
)

$targetDrive = (Split-Path -Qualifier $Target)
$drives = @{}
foreach ($d in [IO.DriveInfo]::GetDrives() | Where-Object { $_.IsReady }) { $drives[$d.Name.Substring(0, 2).ToUpper()] = $d }
Write-Output ("Windows toolchain move ({0})" -f $(if ($Apply) { 'APPLY' } else { 'dry run: nothing is changed' }))
foreach ($k in 'C:', $targetDrive.ToUpper()) {
    if ($drives.ContainsKey($k)) { Write-Output ("  drive {0} {1} free of {2}" -f $k, (Gb $drives[$k].AvailableFreeSpace), (Gb $drives[$k].TotalSize)) }
}
$need = [int64]0
foreach ($m in $moves) {
    $m.size = Size $m.from
    $m.same = ([IO.Path]::GetFullPath($m.from).TrimEnd('\') -ieq [IO.Path]::GetFullPath($m.to).TrimEnd('\'))
    $m.exists = Test-Path -LiteralPath $m.from
    $state = if ($m.same) { 'already there' } elseif (-not $m.exists) { 'not present (only the variable is set)' } else { 'move' }
    Write-Output ("  {0,-12} {1} -> {2}: {3} files, {4} ({5})" -f $m.name, $m.from, $m.to, $m.size.files, (Gb $m.size.bytes), $state)
    if (-not $m.same) { $need += $m.size.bytes }
}
$onC = ($moves | Where-Object { -not $_.same -and $_.from -like 'C:*' } | ForEach-Object { $_.size.bytes } | Measure-Object -Sum).Sum
Write-Output ("  would free on C: {0}; needs {1} on {2}" -f (Gb ([int64]$onC)), (Gb $need), $targetDrive)
$pathOld = Join-Path $cargoOld 'bin'
$pathNew = Join-Path (Join-Path $Target 'cargo') 'bin'
$userPath = UserEnv 'Path'
Write-Output ("  user PATH: {0}" -f $(if ($userPath -and $userPath.Split(';') -contains $pathOld) { "replace $pathOld with $pathNew" } else { "add $pathNew" }))

if (-not $drives.ContainsKey($targetDrive.ToUpper())) { Write-Error "drive $targetDrive not found" }
if ($drives[$targetDrive.ToUpper()].AvailableFreeSpace -lt $need + 10GB) { Write-Error ("not enough space on {0} (needs {1} + 10 GB)" -f $targetDrive, (Gb $need)) }
$busy = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match '^(cargo|rustc|clippy-driver|link|lld-link|rust-lld|rustup|sccache)$' })
if ($busy.Count) { Write-Output ("  running now: {0}" -f (($busy | ForEach-Object { $_.ProcessName } | Sort-Object -Unique) -join ', ')) }
if (-not $Apply) {
    Write-Output "Dry run done. Apply with -Apply (make windows-move-toolchain MACHINE=... APPLY=1)."
    exit 0
}
if ($busy.Count -and -not $Force) { Write-Error 'cargo, rustc, a linker or sccache is running: stop the builds first (or -Force)' }

# sccache's server holds its cache dir: stop it (the next build starts it again).
$sc = Join-Path $env:USERPROFILE '.local\bin\sccache.exe'
if (Test-Path -LiteralPath $sc) { & $sc --stop-server 2>$null | Out-Null }

$saved = @{}
foreach ($n in 'CARGO_HOME', 'RUSTUP_HOME', 'SCCACHE_DIR', 'Path') { $saved[$n] = UserEnv $n }
try {
    foreach ($m in $moves) {
        if ($m.same) { continue }
        New-Item -ItemType Directory -Force -Path $m.to | Out-Null
        if ($m.exists) {
            Write-Output ("copying {0} ..." -f $m.name)
            & robocopy $m.from $m.to /E /COPY:DAT /DCOPY:DAT /R:1 /W:1 /NFL /NDL /NP /NJH /NJS | Out-Null
            if ($LASTEXITCODE -ge 8) { throw "robocopy $($m.name) failed (exit $LASTEXITCODE)" }
            $got = Size $m.to
            if ($got.files -lt $m.size.files -or $got.bytes -lt $m.size.bytes) {
                throw ("copy of {0} incomplete: {1} files / {2} of {3} files / {4}" -f $m.name, $got.files, $got.bytes, $m.size.files, $m.size.bytes)
            }
        }
        [Environment]::SetEnvironmentVariable($m.name, $m.to, 'User')
        Set-Item -Path ("env:" + $m.name) -Value $m.to
    }
    $parts = @(if ($userPath) { $userPath.Split(';') | Where-Object { $_ -and $_ -ne $pathOld -and $_ -ne $pathNew } })
    [Environment]::SetEnvironmentVariable('Path', (@($pathNew) + $parts) -join ';', 'User')
    $env:Path = $pathNew + ';' + (($env:Path.Split(';') | Where-Object { $_ -ne $pathOld }) -join ';')
    # The toolchain from the new place (rustup reads RUSTUP_HOME, cargo CARGO_HOME).
    $cv = & (Join-Path $pathNew 'cargo.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'cargo --version failed from the new place' }
    $rv = & (Join-Path $pathNew 'rustc.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'rustc --version failed from the new place' }
    Write-Output "verified: $cv; $rv"
} catch {
    Write-Output "FAILED: $_ - restoring the user environment, the old directories stay"
    foreach ($n in $saved.Keys) { [Environment]::SetEnvironmentVariable($n, $saved[$n], 'User') }
    exit 1
}
if (-not $KeepOld) {
    foreach ($m in $moves) {
        if ($m.same -or -not $m.exists) { continue }
        Write-Output ("removing the old {0} ({1})" -f $m.name, $m.from)
        Remove-Item -LiteralPath $m.from -Recurse -Force -ErrorAction SilentlyContinue
    }
}
Write-Output 'Done. New SSH sessions use the new place; open a new terminal for interactive use.'
