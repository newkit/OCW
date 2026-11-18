<#
.SYNOPSIS
  Sets up a Windows machine as an OCW build/test server: OpenSSH server, a
  local build user (not an administrator) that logs in with an SSH key, and
  the build tools: Git (with Git LFS) and the Visual Studio Build Tools (C++
  workload) via winget, and rustup for the build user. With -VR it also checks
  the OpenXR runtime for a headset (Meta Quest Link, not on winget) and
  installs ffmpeg (winget Gyan.FFmpeg): VR test runs record the mirror window
  as a video (scripts/vr-soak.ps1, the app's `video` recorder).

.DESCRIPTION
  Works on any Windows language (German, English, ...): built-in groups are
  addressed by their SIDs, not their translated names. Safe to run again:
  existing parts are kept, the key is only added once.

  Run in PowerShell as Administrator, e.g.:

    Set-ExecutionPolicy -Scope Process Bypass
    .\setup-windows-buildserver.ps1 -PublicKey 'ssh-ed25519 AAAA... ocw-windows'

  or with the key file copied to the machine:

    .\setup-windows-buildserver.ps1 -PublicKeyFile .\ocw_windows_ed25519.pub

  For a machine with a VR headset (Meta Quest over USB / Link):

    .\setup-windows-buildserver.ps1 -PublicKey '...' -VR

  Afterwards (on the Linux dev machine), see config/README.md:
    ssh-keyscan <ip> >> config/ssh/known_hosts
    and add the machine to config/machines.toml (user = "claude", shell = "powershell").

.PARAMETER User
  Name of the local build user (default: claude).

.PARAMETER PublicKey
  The SSH public key line (contents of the .pub file).

.PARAMETER PublicKeyFile
  Path of the .pub file instead of -PublicKey.

.PARAMETER RepoPath
  Where the repository is copied to (repo_path in machines.toml); made
  writable for the build user. Default: C:\src\OCW.

.PARAMETER Password
  Password of the build user (asked for if not given and the user is new, or
  to install rustup as the build user).

.PARAMETER SkipTools
  Only SSH and the user: do not install Git, the Build Tools and rustup.

.PARAMETER VR
  Also the VR side: check the active OpenXR runtime (Meta Quest Link or
  SteamVR) and print what is missing; install ffmpeg (Gyan.FFmpeg) for the
  videos of VR test runs.
#>
[CmdletBinding()]
param(
    [string]$User = "claude",
    [string]$PublicKey,
    [string]$PublicKeyFile,
    [string]$RepoPath = "C:\src\OCW",
    [securestring]$Password,
    [switch]$SkipTools,
    [switch]$VR
)

$ErrorActionPreference = "Stop"

# Built-in SIDs, the same in every Windows language.
$SidUsers  = "S-1-5-32-545"   # Users / Benutzer
$SidAdmins = "S-1-5-32-544"   # Administrators / Administratoren
$SidSystem = "S-1-5-18"       # SYSTEM

function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }

# --- Checks -------------------------------------------------------------------
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.SecurityIdentifier]$SidAdmins)) {
    throw "Please run this script in PowerShell as Administrator."
}
if ($PublicKeyFile) { $PublicKey = (Get-Content -Raw $PublicKeyFile).Trim() }
if (-not $PublicKey) { throw "Give the SSH public key: -PublicKey 'ssh-ed25519 AAAA...' or -PublicKeyFile file.pub" }
if ($PublicKey -notmatch '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-\S+) \S+') {
    throw "That does not look like an SSH public key (expected 'ssh-ed25519 AAAA...'). Never pass the private key."
}

# --- 1. OpenSSH server ---------------------------------------------------------
Step "OpenSSH server"
$cap = Get-WindowsCapability -Online | Where-Object Name -like 'OpenSSH.Server*' | Select-Object -First 1
if ($cap.State -ne 'Installed') {
    Add-WindowsCapability -Online -Name $cap.Name | Out-Null
    Write-Host "   installed $($cap.Name)"
} else {
    Write-Host "   already installed"
}
Set-Service -Name sshd -StartupType Automatic
Start-Service sshd

Step "Firewall rule for port 22"
if (-not (Get-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -ErrorAction SilentlyContinue)) {
    New-NetFirewallRule -Name 'OpenSSH-Server-In-TCP' -DisplayName 'OpenSSH Server (sshd)' `
        -Enabled True -Direction Inbound -Protocol TCP -Action Allow -LocalPort 22 | Out-Null
    Write-Host "   rule created"
} else {
    Enable-NetFirewallRule -Name 'OpenSSH-Server-In-TCP'
    Write-Host "   rule present"
}

Step "PowerShell as the SSH login shell"
New-ItemProperty -Path 'HKLM:\SOFTWARE\OpenSSH' -Name DefaultShell -Force -PropertyType String `
    -Value "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" | Out-Null

# --- 2. Local build user -------------------------------------------------------
Step "Local user '$User'"
$existing = Get-LocalUser -Name $User -ErrorAction SilentlyContinue
if (-not $existing) {
    if (-not $Password) { $Password = Read-Host -AsSecureString "Password for $User" }
    New-LocalUser -Name $User -Password $Password -FullName "OCW Build" `
        -Description "Open Cycling World build/test user" `
        -PasswordNeverExpires -AccountNeverExpires | Out-Null
    Write-Host "   created"
} else {
    Write-Host "   exists"
}
$userSid = (Get-LocalUser -Name $User).SID.Value
$inUsers = Get-LocalGroupMember -SID $SidUsers | Where-Object { $_.SID.Value -eq $userSid }
if (-not $inUsers) { Add-LocalGroupMember -SID $SidUsers -Member $User }
$inAdmins = Get-LocalGroupMember -SID $SidAdmins | Where-Object { $_.SID.Value -eq $userSid }
if ($inAdmins) {
    Write-Warning "$User is an administrator: Windows then ignores its own authorized_keys and only reads C:\ProgramData\ssh\administrators_authorized_keys. A normal user is recommended."
}

Step "Profile folder"
$userProfile = (Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $userSid }).LocalPath
if (-not $userProfile) {
    # Create the profile by running a program as the user once. It starts in
    # the Windows folder: by default it would start in the current folder
    # (e.g. the admin's Downloads), which the new user cannot open, and
    # Start-Process then fails with "The directory name is invalid".
    if (-not $Password) { $Password = Read-Host -AsSecureString "Password for $User (to create its profile)" }
    $cred = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$User", $Password)
    Start-Process -FilePath "$env:SystemRoot\System32\cmd.exe" -ArgumentList '/c exit' `
        -WorkingDirectory $env:SystemRoot -Credential $cred -LoadUserProfile -Wait -WindowStyle Hidden
    $userProfile = (Get-CimInstance Win32_UserProfile | Where-Object { $_.SID -eq $userSid }).LocalPath
}
if (-not $userProfile) { throw "No profile folder for $User; log in once as $User and run the script again." }
Write-Host "   $userProfile"

Step "SSH key"
$ssh = Join-Path $userProfile ".ssh"
$keys = Join-Path $ssh "authorized_keys"
New-Item -ItemType Directory -Force $ssh | Out-Null
$present = (Test-Path $keys) -and ((Get-Content $keys) -contains $PublicKey)
if (-not $present) {
    # ASCII without a byte-order mark: sshd rejects keys files with one.
    Add-Content -Encoding ascii -Path $keys -Value $PublicKey
    Write-Host "   key added"
} else {
    Write-Host "   key already present"
}
icacls $ssh  /inheritance:r /grant "*${userSid}:(OI)(CI)F" /grant "*${SidSystem}:(OI)(CI)F" /grant "*${SidAdmins}:(OI)(CI)F" | Out-Null
icacls $keys /inheritance:r /grant "*${userSid}:F" /grant "*${SidSystem}:F" /grant "*${SidAdmins}:F" | Out-Null

Step "Repository folder $RepoPath"
New-Item -ItemType Directory -Force $RepoPath | Out-Null
icacls $RepoPath /grant "*${userSid}:(OI)(CI)M" | Out-Null

Restart-Service sshd

# --- Build tools ---------------------------------------------------------------
function Test-Winget($id) {
    [bool](winget list --id $id -e --accept-source-agreements 2>$null | Select-String -SimpleMatch $id)
}
function Install-Winget($id, [string[]]$extra) {
    if (Test-Winget $id) { Write-Host "   $id already installed"; return }
    Write-Host "   installing $id (this can take a while)"
    winget install --id $id -e --silent --accept-source-agreements --accept-package-agreements @extra
    # winget's exit code is not reliable (e.g. a pending restart): check the result instead.
    if (-not (Test-Winget $id)) { throw "winget could not install $id" }
}

if (-not $SkipTools) {
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw "winget is missing: install 'App Installer' from the Microsoft Store, then run the script again (or use -SkipTools)."
    }
    Step "Git (with Git LFS)"
    Install-Winget "Git.Git" @("--scope", "machine")
    $git = "$env:ProgramFiles\Git\cmd\git.exe"
    if (Test-Path $git) { & $git lfs install --system | Out-Null; Write-Host "   git lfs enabled for all users" }

    Step "Visual Studio Build Tools (C++ workload)"
    Install-Winget "Microsoft.VisualStudio.2022.BuildTools" @("--override",
        "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended")

    Step "rustup for $User"
    $rustc = Join-Path $userProfile ".cargo\bin\rustc.exe"
    if (Test-Path $rustc) {
        Write-Host "   already installed"
    } else {
        # rustup installs per user: run its installer as the build user, from a
        # folder that user can read, starting in the Windows folder (see the
        # Profile folder step).
        if (-not $Password) { $Password = Read-Host -AsSecureString "Password for $User (to install rustup as $User)" }
        $cred = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$User", $Password)
        $dir = Join-Path $env:ProgramData "ocw-setup"
        New-Item -ItemType Directory -Force $dir | Out-Null
        $init = Join-Path $dir "rustup-init.exe"
        $ProgressPreference = "SilentlyContinue"
        Invoke-WebRequest -UseBasicParsing "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe" -OutFile $init
        $p = Start-Process -FilePath $init -ArgumentList "-y --default-toolchain stable --profile minimal" `
            -WorkingDirectory $env:SystemRoot -Credential $cred -LoadUserProfile -Wait -WindowStyle Hidden -PassThru
        Remove-Item $init -ErrorAction SilentlyContinue
        if ($p.ExitCode -ne 0 -or -not (Test-Path $rustc)) { throw "rustup-init failed for $User (exit $($p.ExitCode))" }
        Write-Host "   installed (the toolchain of rust-toolchain.toml comes with the first build)"
    }
}

# --- VR ------------------------------------------------------------------------
$vrMissing = @()
if ($VR) {
    Step "VR: OpenXR runtime"
    $active = (Get-ItemProperty -Path "HKLM:\SOFTWARE\Khronos\OpenXR\1" -Name ActiveRuntime -ErrorAction SilentlyContinue).ActiveRuntime
    if ($active) {
        Write-Host "   active runtime: $active"
        if ($active -notmatch "oculus|meta|steamvr") { Write-Warning "Unknown OpenXR runtime; OCW is tested with Meta Quest Link and SteamVR." }
    } else {
        $vrMissing += "Install the Meta Quest Link app (https://www.meta.com/help/quest/pcvr/), log in, connect the headset by USB once and set Link as the active OpenXR runtime (Settings > General > OpenXR Runtime). SteamVR is the alternative."
        Write-Warning "No active OpenXR runtime."
    }
    # Every test run records pictures and, where it moves, a video
    # (CLAUDE.md): the VR runs film the mirror window with the app's own
    # recorder, which streams into ffmpeg. Installed for all users here (an
    # elevated script cannot run winget in the build user's context); on
    # Weela it was installed in the build user's scope (winget install
    # Gyan.FFmpeg as that user) - vr-soak.ps1 finds either.
    Step "VR: ffmpeg (videos of the VR test runs)"
    if ($SkipTools) {
        $vrMissing += "Install ffmpeg (winget install --id Gyan.FFmpeg -e) for the videos of VR test runs."
    } else {
        Install-Winget "Gyan.FFmpeg" @("--scope", "machine")
    }
}

# --- Summary -------------------------------------------------------------------
$ip = (Get-NetIPAddress -AddressFamily IPv4 | Where-Object {
        $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
    Select-Object -First 1).IPAddress
$profileName = (Get-NetConnectionProfile | Select-Object -First 1).NetworkCategory
Write-Host ""
Write-Host "Done." -ForegroundColor Green
Write-Host "  user:      $env:COMPUTERNAME\$User"
Write-Host "  address:   $ip"
Write-Host "  repo_path: $RepoPath"
if ($profileName -eq 'Public') {
    Write-Warning "The network is 'Public' (Oeffentlich); if SSH does not connect, set it to 'Private'."
}
Write-Host ""
if ($SkipTools) {
    Write-Host "Build tools skipped (-SkipTools): install Git, the Visual Studio Build Tools (C++ workload)"
    Write-Host "and rustup as $User, e.g. 'make deps-windows' (see config/README.md)."
}
foreach ($m in $vrMissing) { Write-Host "VR: $m" -ForegroundColor Yellow }
Write-Host "GUI runs (screenshots, the app, VR) need $User logged in on the desktop."
