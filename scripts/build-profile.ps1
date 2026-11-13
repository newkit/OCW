# Build profiling on Windows (spec §11.2): the PowerShell counterpart of
# scripts/build-profile.sh for the build machine (no bash there), e.g.
#
#   scripts/remote.sh Win11Builder run 'powershell -ExecutionPolicy Bypass -File scripts/build-profile.ps1 -Label win-after'
#   scripts/remote.sh Win11Builder fetch test-output/build-profile test-output
#
#   -Label     name of the run; results in test-output/build-profile/LABEL/
#   -Commands  subset of build,test,clippy (default: build)
#   -FullDebug build with the full debug info of before v2.59 (profile.dev
#              debug = true for all crates), for before/after comparisons
#
# Steps per command: clean (empty target directory), noop, touch and edit
# (append a private function) of crates/ocw-world/src/lib.rs. Writes
# results.jsonl (as build-profile.sh) and the --timings reports. The
# target directory (target\build-profile) is removed afterwards.
param(
    [string]$Label = 'windows',
    [string]$Commands = 'build',
    [switch]$FullDebug
)
$ErrorActionPreference = 'Continue'
$out = Join-Path 'test-output\build-profile' $Label
New-Item -ItemType Directory -Force -Path $out | Out-Null
$results = Join-Path $out 'results.jsonl'
$tdir = 'target\build-profile'
$env:CARGO_TARGET_DIR = $tdir
$config = @()
if ($FullDebug) {
    $config = @('--config', 'profile.dev.debug=true', '--config', "profile.dev.package.'*'.debug=true")
}
$args_of = @{
    build  = @('build', '--workspace')
    test   = @('test', '--workspace', '--no-run')
    clippy = @('clippy', '--workspace', '--all-targets')
}

function Run($cmd, $step) {
    $log = Join-Path $out "$cmd-$step.log"
    $a = $args_of[$cmd] + $config + @('--timings')
    $t = [Diagnostics.Stopwatch]::StartNew()
    & cargo @a *> $log
    $code = $LASTEXITCODE
    $secs = [math]::Round($t.Elapsed.TotalSeconds, 1)
    $compiled = @(Select-String -Path $log -Pattern '^\s*(Compiling|Checking) ').Count
    $html = Get-ChildItem "$tdir\cargo-timings\cargo-timing-*.html" -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime | Select-Object -Last 1
    if ($html) { Copy-Item $html.FullName (Join-Path $out "$cmd-$step.html") }
    $mb = 'null'
    if ($step -eq 'clean') {
        $mb = [math]::Round(((Get-ChildItem $tdir -Recurse -File -ErrorAction SilentlyContinue |
            Measure-Object Length -Sum).Sum) / 1MB)
    }
    $line = '{"label":"' + $Label + '","command":"' + $cmd + '","step":"' + $step + '","seconds":' +
        $secs.ToString([Globalization.CultureInfo]::InvariantCulture) + ',"compiled":' + $compiled +
        ',"exit":' + $code + ',"load":0,"jobs":"' + $env:CARGO_BUILD_JOBS + '","wrapper":"","config":"' +
        ($(if ($FullDebug) { 'full debug' } else { '' })) + '","target_mb":' + $mb + '}'
    Add-Content -Path $results -Value $line
    Write-Output ("build-profile: {0,-8} {1,-16} {2,8}s {3,5} crates exit {4}" -f $cmd, $step, $secs, $compiled, $code)
}

$f = 'crates\ocw-world\src\lib.rs'
foreach ($c in $Commands.Split(',')) {
    if (Test-Path $tdir) { Remove-Item -Recurse -Force $tdir }
    Run $c 'clean'
    Run $c 'noop'
    (Get-Item $f).LastWriteTime = Get-Date
    Run $c 'touch-ocw-world'
    $orig = [IO.File]::ReadAllText($f)
    [IO.File]::WriteAllText($f, $orig + "`n#[allow(dead_code)]`nfn build_profile_probe() -> u32 {`n    7`n}`n")
    Run $c 'edit-ocw-world'
    [IO.File]::WriteAllText($f, $orig)
    if (Test-Path $tdir) { Remove-Item -Recurse -Force $tdir }
}
