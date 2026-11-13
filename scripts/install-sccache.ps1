# Installs a pinned, SHA-256-checked sccache (MSVC build) to
# %USERPROFILE%\.local\bin\sccache.exe on a Windows build machine (spec
# §11.2); scripts/remote.sh uses it from there. Called by `make
# deps-sccache-windows MACHINE=...` (the version and checksum live in the
# Makefile).
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$Sha256
)
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$name = "sccache-v$Version-x86_64-pc-windows-msvc"
$bin = Join-Path $env:USERPROFILE '.local\bin'
$exe = Join-Path $bin 'sccache.exe'
if ((Test-Path $exe) -and ((& $exe --version) -eq "sccache $Version")) {
    Write-Output "sccache $Version already installed: $exe"
    exit 0
}
New-Item -ItemType Directory -Force -Path $bin | Out-Null
$tmp = Join-Path $env:TEMP "ocw-sccache-$PID"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
try {
    $zip = Join-Path $tmp "$name.zip"
    Invoke-WebRequest -UseBasicParsing -OutFile $zip `
        -Uri "https://github.com/mozilla/sccache/releases/download/v$Version/$name.zip"
    $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLowerInvariant()
    if ($got -ne $Sha256.ToLowerInvariant()) {
        Write-Error "sccache download: SHA-256 $got, expected $Sha256"
        exit 1
    }
    Expand-Archive -Force -LiteralPath $zip -DestinationPath $tmp
    $found = Get-ChildItem -Path $tmp -Recurse -Filter 'sccache.exe' | Select-Object -First 1
    if (-not $found) { Write-Error "no sccache.exe in $name.zip"; exit 1 }
    Copy-Item -Force -LiteralPath $found.FullName -Destination $exe
    & $exe --version
} finally {
    Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
}
