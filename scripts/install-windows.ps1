<#
.SYNOPSIS
Install, upgrade or remove Tend for Windows for the current user.

.DESCRIPTION
Downloads the release zip for x86_64-pc-windows-msvc, checks it against its
published SHA-256, and installs tend.exe, tendd.exe and tend-app.exe into a
per-user folder (no administrator rights). Then it:

  - puts that folder on the user PATH;
  - registers a Task Scheduler task, tend-daemon, that starts tendd at logon;
  - turns on the local dashboard the app talks to (tend init --component dashboard);
  - starts tendd, and starts tend-app with Windows (tend-app --install);
  - adds a Tend shortcut to the Start menu.

Running it again upgrades in place: it stops tend-app and tendd from the
install folder, replaces the binaries and starts both again.

-Uninstall reverses all of it except your data and settings
(%APPDATA%\tend, %LOCALAPPDATA%\tend), which it lists.

The binaries are not code-signed. See docs/WINDOWS.md for what Windows shows
about that.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File install-tend.ps1

.EXAMPLE
powershell -ExecutionPolicy Bypass -File install-tend.ps1 -Version v0.10.24 -NoApp

.EXAMPLE
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\tend\install-tend.ps1" -Uninstall
#>
[CmdletBinding()]
param(
    # A release tag such as v0.10.24, or 'latest'.
    [string]$Version = 'latest',
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\tend'),
    # Install from a local zip instead of downloading. Its .sha256 sidecar
    # (the one the release publishes) must sit next to it.
    [string]$Archive,
    # Only the command line and the daemon: no tend-app, no dashboard change.
    [switch]$NoApp,
    # No Start menu shortcut.
    [switch]$NoShortcut,
    # Install only; do not start tendd or tend-app now.
    [switch]$NoStart,
    [switch]$Uninstall
)

Set-StrictMode -Version 2
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Repo = 'Open330/tend'
$Target = 'x86_64-pc-windows-msvc'
$TaskName = 'tend-daemon'
$Binaries = @('tend.exe', 'tendd.exe', 'tend-app.exe')
$ScriptName = 'install-tend.ps1'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Tend.lnk'

function Say([string]$text) { Write-Host "tend: $text" }
function Warn([string]$text) { Write-Warning $text }

function SamePath([string]$a, [string]$b) {
    $a.TrimEnd('\') -ieq $b.TrimEnd('\')
}

function Inside([string]$path, [string]$dir) {
    $path -and $path.StartsWith($dir.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
}

function IsElevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# --- PATH -----------------------------------------------------------------
# Read and write the raw registry value so entries like %USERPROFILE%\bin
# keep their REG_EXPAND_SZ form (Environment.GetEnvironmentVariable would
# expand them and write them back literal).
function UserPath {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment')
    try {
        $raw = $key.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
        $kind = 'ExpandString'
        if ($raw) { $kind = $key.GetValueKind('Path') }
        return @{ Raw = [string]$raw; Kind = $kind }
    } finally { $key.Close() }
}

function SetUserPath([string]$value, $kind) {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    try { $key.SetValue('Path', $value, $kind) } finally { $key.Close() }
    # Setting any user variable through .NET broadcasts WM_SETTINGCHANGE, so
    # Explorer (and terminals started from it) see the new PATH.
    [Environment]::SetEnvironmentVariable('TEND_INSTALL_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('TEND_INSTALL_REFRESH', $null, 'User')
}

function AddToPath([string]$dir) {
    $current = UserPath
    $entries = @($current.Raw -split ';' | Where-Object { $_ })
    if ($entries | Where-Object { SamePath ([Environment]::ExpandEnvironmentVariables($_)) $dir }) {
        return
    }
    SetUserPath ((@($entries) + $dir) -join ';') $current.Kind
    Say "added $dir to your PATH (new terminals see it)"
}

# A new process's PATH is the machine PATH, then the user PATH. Name any
# other tend.exe that comes first, since that is the one a terminal runs.
function WarnShadowed([string]$dir) {
    $machine = [Environment]::GetEnvironmentVariable('Path', 'Machine')
    foreach ($entry in (($machine + ';' + (UserPath).Raw) -split ';')) {
        if (-not $entry) { continue }
        $expanded = [Environment]::ExpandEnvironmentVariables($entry)
        if (SamePath $expanded $dir) { return }
        if (Test-Path -LiteralPath (Join-Path $expanded 'tend.exe')) {
            Warn "another tend.exe comes first on PATH: $expanded. Remove it, or that folder from PATH, so terminals run this install."
            return
        }
    }
}

function RemoveFromPath([string]$dir) {
    $current = UserPath
    $entries = @($current.Raw -split ';' | Where-Object { $_ })
    $kept = @($entries | Where-Object { -not (SamePath ([Environment]::ExpandEnvironmentVariables($_)) $dir) })
    if ($kept.Count -ne $entries.Count) {
        SetUserPath ($kept -join ';') $current.Kind
        Say "removed $dir from your PATH"
    }
}

# --- processes ------------------------------------------------------------
function RunningFrom([string]$name, [string]$dir) {
    @(Get-Process -Name $name -ErrorAction SilentlyContinue | Where-Object {
        try { Inside $_.Path $dir } catch { $false }
    })
}

function StopTend([string]$dir) {
    foreach ($p in (RunningFrom 'tend-app' $dir)) {
        Say "stopping tend-app (pid $($p.Id))"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
    $tend = Join-Path $dir 'tend.exe'
    if (@(RunningFrom 'tendd' $dir).Count -gt 0 -and (Test-Path $tend)) {
        Say 'stopping tendd'
        & $tend daemon stop 2>&1 | Out-Null
    }
    $deadline = (Get-Date).AddSeconds(15)
    while (@(RunningFrom 'tendd' $dir).Count -gt 0 -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
    }
    foreach ($p in (RunningFrom 'tendd' $dir)) {
        Warn "tendd (pid $($p.Id)) did not stop; ending it"
        Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    }
}

# --- the logon task -------------------------------------------------------
# conhost --headless keeps a console window from flashing at logon. The task
# runs at the desktop's level (Limited) even from an elevated shell, so the
# daemon sees the same rmux server as the terminals you open. tend daemon
# start creates tendd through Win32_Process.Create, outside the task's job,
# so tendd outlives the task.
function RegisterDaemonTask([string]$tend) {
    $conhost = Join-Path $env:SystemRoot 'System32\conhost.exe'
    $user = [Security.Principal.WindowsIdentity]::GetCurrent().Name
    $action = New-ScheduledTaskAction -Execute $conhost -Argument "--headless `"$tend`" daemon start"
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User $user
    $principal = New-ScheduledTaskPrincipal -UserId $user -LogonType Interactive -RunLevel Limited
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 5) -MultipleInstances IgnoreNew
    Register-ScheduledTask -TaskName $TaskName -Description 'Start tendd at logon (installed by install-tend.ps1)' `
        -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Say "registered the logon task $TaskName"
}

function StartDaemon([string]$dir) {
    try { Start-ScheduledTask -TaskName $TaskName } catch { }
    $deadline = (Get-Date).AddSeconds(15)
    while (@(RunningFrom 'tendd' $dir).Count -eq 0 -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
    }
    if (@(RunningFrom 'tendd' $dir).Count -gt 0) {
        Say 'tendd is running'
        return
    }
    # No interactive session for the task to run in (for example over ssh
    # with nobody signed in): start it from here instead.
    & (Join-Path $dir 'tend.exe') daemon start
    if ($LASTEXITCODE -ne 0) { Warn "tend daemon start exited with $LASTEXITCODE; see 'tend daemon logs'" }
}

# --- download and verify --------------------------------------------------
function ResolveTag([string]$version) {
    if ($version -and $version -ne 'latest') {
        if ($version.StartsWith('v')) { return $version }
        return "v$version"
    }
    $release = Invoke-RestMethod -UseBasicParsing -Uri "https://api.github.com/repos/$Repo/releases/latest" `
        -Headers @{ 'User-Agent' = 'tend-install' }
    return [string]$release.tag_name
}

function Expected([string]$sidecar) {
    $line = (Get-Content -LiteralPath $sidecar -TotalCount 1).Trim()
    $hash = ($line -split '\s+')[0].ToLowerInvariant()
    if ($hash -notmatch '^[0-9a-f]{64}$') { throw "no SHA-256 in $sidecar" }
    return $hash
}

function FetchArchive([string]$work) {
    if ($Archive) {
        $zip = (Resolve-Path -LiteralPath $Archive).Path
        $sidecar = "$zip.sha256"
        if (-not (Test-Path -LiteralPath $sidecar)) { throw "$sidecar is missing; it must sit next to the zip" }
        return @{ Zip = $zip; Sidecar = $sidecar }
    }
    $tag = ResolveTag $Version
    $name = "tend-$tag-$Target.zip"
    $base = "https://github.com/$Repo/releases/download/$tag"
    $zip = Join-Path $work $name
    Say "downloading $name"
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $zip
    Invoke-WebRequest -UseBasicParsing -Uri "$base/$name.sha256" -OutFile "$zip.sha256"
    return @{ Zip = $zip; Sidecar = "$zip.sha256" }
}

# --- install / uninstall --------------------------------------------------
function Install {
    if (IsElevated) {
        Warn 'this PowerShell is elevated. Tend installs for your user only; tend-app started from here runs elevated. Prefer a normal PowerShell.'
    }
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        Warn "this is a $($env:PROCESSOR_ARCHITECTURE) machine; the x64 build runs under emulation"
    }
    $work = Join-Path ([IO.Path]::GetTempPath()) ("tend-install-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        $fetched = FetchArchive $work
        $want = Expected $fetched.Sidecar
        $got = (Get-FileHash -LiteralPath $fetched.Zip -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            throw "checksum mismatch for $(Split-Path -Leaf $fetched.Zip): expected $want, got $got. Nothing was installed."
        }
        Say "checksum ok ($want)"

        $unpacked = Join-Path $work 'unpacked'
        Expand-Archive -LiteralPath $fetched.Zip -DestinationPath $unpacked
        $wanted = $Binaries
        if ($NoApp) { $wanted = @('tend.exe', 'tendd.exe') }
        $files = @{}
        foreach ($bin in $wanted + @($ScriptName, 'install-windows.ps1')) {
            $found = Get-ChildItem -LiteralPath $unpacked -Recurse -File -Filter $bin | Select-Object -First 1
            if ($found) { $files[$bin] = $found.FullName }
        }
        foreach ($bin in $wanted) {
            if (-not $files.ContainsKey($bin)) { throw "$bin is not in the archive" }
        }

        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
        StopTend $InstallDir
        foreach ($bin in $wanted) {
            $dest = Join-Path $InstallDir $bin
            $aside = "$dest.old"
            Remove-Item -LiteralPath $aside -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $dest) {
                # Windows refuses to overwrite a running executable but lets
                # one be renamed; a stray process from elsewhere cannot block.
                try { Remove-Item -LiteralPath $dest -Force } catch { Move-Item -LiteralPath $dest -Destination $aside -Force }
            }
            Copy-Item -LiteralPath $files[$bin] -Destination $dest
            Unblock-File -LiteralPath $dest -ErrorAction SilentlyContinue
        }
        $self = $files[$ScriptName]
        if (-not $self) { $self = $files['install-windows.ps1'] }
        if (-not $self -and $PSCommandPath) { $self = $PSCommandPath }
        if ($self -and -not (SamePath $self (Join-Path $InstallDir $ScriptName))) {
            Copy-Item -LiteralPath $self -Destination (Join-Path $InstallDir $ScriptName) -Force
        }
        Say "installed $($wanted -join ', ') into $InstallDir"
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    AddToPath $InstallDir
    WarnShadowed $InstallDir
    $env:Path = "$InstallDir;$env:Path"
    $tend = Join-Path $InstallDir 'tend.exe'
    $app = Join-Path $InstallDir 'tend-app.exe'
    $version = (& $tend --version) 2>$null
    if ($version) { Say "$version" }

    if (-not (Get-Command rmux -ErrorAction SilentlyContinue)) {
        Warn 'rmux is not on PATH. tendd runs without it, but sees no panes until it is installed: winget install Helvesec.RMUX'
    }

    RegisterDaemonTask $tend
    if (-not $NoApp) {
        # The app needs the loopback dashboard with a token; this keeps an
        # existing [dashboard] section as it is.
        & $tend init --component dashboard --yes --start-daemon=false
        if ($LASTEXITCODE -ne 0) { Warn "tend init --component dashboard exited with $LASTEXITCODE" }
        if (-not $NoShortcut) {
            $shell = New-Object -ComObject WScript.Shell
            $link = $shell.CreateShortcut($Shortcut)
            $link.TargetPath = $app
            $link.WorkingDirectory = $InstallDir
            $link.Description = 'Tend'
            $link.Save()
            Say "added the Start menu shortcut $Shortcut"
        }
    }
    if ($NoStart) { Say 'done (not started: -NoStart)'; return }
    StartDaemon $InstallDir
    if (-not $NoApp) {
        # Registers toasts and the start-with-Windows entry, then runs the app.
        Start-Process -FilePath $app -ArgumentList '--install' -WorkingDirectory $InstallDir
        Say 'started tend-app; it is in the notification area (Windows may put it under the ^ arrow)'
    }
    Say 'done. In a new terminal, run tend init (inside an rmux pane) to wire agent hooks and key bindings.'
}

function Remove {
    $app = Join-Path $InstallDir 'tend-app.exe'
    if (Test-Path -LiteralPath $app) {
        # Removes the start-with-Windows entry and the toast registration.
        $p = Start-Process -FilePath $app -ArgumentList '--uninstall' -PassThru -Wait
        if ($p.ExitCode -ne 0) { Warn "tend-app --uninstall exited with $($p.ExitCode)" }
    }
    StopTend $InstallDir
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $Shortcut -Force -ErrorAction SilentlyContinue
    RemoveFromPath $InstallDir
    if (Test-Path -LiteralPath $InstallDir) {
        Remove-Item -LiteralPath $InstallDir -Recurse -Force
        Say "removed $InstallDir"
    }
    Say 'uninstalled. Kept your settings and data:'
    foreach ($dir in @((Join-Path $env:APPDATA 'tend'), (Join-Path $env:LOCALAPPDATA 'tend'))) {
        if (Test-Path -LiteralPath $dir) { Say "  $dir" }
    }
    Say 'tend init --uninstall (run before this) also removes the hooks and rmux bindings tend init added.'
}

[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
if ($Uninstall) { Remove } else { Install }
