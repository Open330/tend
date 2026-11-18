<#
.SYNOPSIS
Install, upgrade or remove Tend for Windows for the current user.

.DESCRIPTION
Downloads the release zip for x86_64-pc-windows-msvc, checks it against its
published SHA-256, and installs tend.exe, tendd.exe and tend-app.exe into a
per-user folder (no administrator rights). Then it:

  - puts that folder on the user PATH;
  - registers a Task Scheduler task, tend-daemon-<your SID>, that starts
    tendd at logon;
  - turns on the local dashboard the app talks to (tend init --component dashboard);
  - starts tendd, and starts tend-app with Windows (tend-app --install);
  - adds a Tend shortcut to the Start menu.

It records what it created in install-tend.json in the install folder.
Running the installed copy again upgrades that install in place: it stops
tend-app and tendd from the folder, replaces the binaries and starts both.

-Uninstall removes only what install-tend.json lists, and keeps your data
and settings (%APPDATA%\tend, %LOCALAPPDATA%\tend), which it names.

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
    # An absolute path. Defaults to the folder of an installed copy of this
    # script, else %LOCALAPPDATA%\Programs\tend. It must be new, empty, or
    # an earlier install by this script.
    [string]$InstallDir,
    # Install from a local zip instead of downloading. Its checksum file
    # (tend-<tag>-x86_64-pc-windows-msvc.sha256, as the release publishes
    # it) must sit next to it.
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
$Bound = $PSBoundParameters
$ProgressPreference = 'SilentlyContinue'

$Repo = 'Open330/tend'
$Target = 'x86_64-pc-windows-msvc'
$Sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$TaskName = "tend-daemon-$Sid"
$ScriptName = 'install-tend.ps1'
$ManifestName = 'install-tend.json'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunValue = 'Tend'
$ShortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'Tend.lnk'

function Say([string]$text) { Write-Host "tend: $text" }
function Warn([string]$text) { Write-Warning $text }

function SamePath([string]$a, [string]$b) {
    $a.TrimEnd('\') -ieq $b.TrimEnd('\')
}

function Inside([string]$path, [string]$dir) {
    $path -and $path.StartsWith($dir.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)
}

function Mentions([string]$text, [string]$path) {
    $text -and $text.IndexOf($path, [StringComparison]::OrdinalIgnoreCase) -ge 0
}

function IsElevated {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    ([Security.Principal.WindowsPrincipal]$id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

# --- where -----------------------------------------------------------------
# An installed copy (a manifest next to it) works on its own install unless
# told otherwise; a downloaded copy uses the default folder.
function ResolveInstallDir {
    $dir = $InstallDir
    if (-not $dir) {
        if ($PSScriptRoot -and (Test-Path -LiteralPath (Join-Path $PSScriptRoot $ManifestName))) {
            $dir = $PSScriptRoot
        } else {
            $dir = Join-Path $env:LOCALAPPDATA 'Programs\tend'
        }
    }
    if ($dir -notmatch '^[A-Za-z]:\\' -and $dir -notmatch '^\\\\[^\\]+\\[^\\]+') {
        throw "-InstallDir must be an absolute path such as D:\Apps\Tend (got '$dir')"
    }
    return [IO.Path]::GetFullPath($dir).TrimEnd('\')
}

function ReadManifest([string]$dir) {
    $path = Join-Path $dir $ManifestName
    if (-not (Test-Path -LiteralPath $path)) { return $null }
    $manifest = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
    if ($manifest.sid -ne $Sid) {
        throw "$path belongs to another Windows user ($($manifest.sid)); nothing was changed"
    }
    return $manifest
}

function WriteManifest([string]$dir, $manifest) {
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $dir $ManifestName) -Encoding UTF8
}

# --- environment ------------------------------------------------------------
# tend honours TEND_SOCKET, TEND_CONFIG and the XDG directories. A shell
# pointed at a scratch daemon must not make the installer configure, stop or
# start that one: every tend call here sees what the logon task sees, the
# user's and the machine's own values.
$EndpointVariables = @('TEND_SOCKET', 'TEND_CONFIG', 'TEND_RMUX_SOCKET', 'XDG_CONFIG_HOME', 'XDG_DATA_HOME', 'XDG_STATE_HOME')
foreach ($name in $EndpointVariables) {
    $value = [Environment]::GetEnvironmentVariable($name, 'User')
    if (-not $value) { $value = [Environment]::GetEnvironmentVariable($name, 'Machine') }
    [Environment]::SetEnvironmentVariable($name, $value, 'Process')
}

# --- PATH -------------------------------------------------------------------
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

function OnPath([string]$entry, [string]$dir) {
    SamePath ([Environment]::ExpandEnvironmentVariables($entry)) $dir
}

function AddToPath([string]$dir) {
    $current = UserPath
    $entries = @($current.Raw -split ';' | Where-Object { $_ })
    if (@($entries | Where-Object { OnPath $_ $dir }).Count -gt 0) { return }
    SetUserPath ((@($entries) + $dir) -join ';') $current.Kind
    Say "added $dir to your PATH (new terminals see it)"
}

function RemoveFromPath([string]$dir) {
    $current = UserPath
    $entries = @($current.Raw -split ';' | Where-Object { $_ })
    $kept = @($entries | Where-Object { -not (OnPath $_ $dir) })
    if ($kept.Count -ne $entries.Count) {
        SetUserPath ($kept -join ';') $current.Kind
        Say "removed $dir from your PATH"
    }
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

# --- processes --------------------------------------------------------------
function RunningFrom([string]$name, [string]$dir) {
    @(Get-CimInstance Win32_Process -Filter "Name='$name.exe'" | Where-Object { Inside $_.ExecutablePath $dir })
}

# Stop what runs from this install, and only that: the daemon is asked to
# stop on the socket its own command line names, so a shell aimed at another
# daemon cannot redirect the request.
function StopTend([string]$dir) {
    foreach ($p in (RunningFrom 'tend-app' $dir)) {
        Say "stopping tend-app (pid $($p.ProcessId))"
        Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
    }
    $tend = Join-Path $dir 'tend.exe'
    foreach ($p in (RunningFrom 'tendd' $dir)) {
        $socket = $null
        if ($p.CommandLine -match '"--socket"\s+"([^"]+)"') { $socket = $Matches[1] }
        elseif ($p.CommandLine -match '--socket\s+(\S+)') { $socket = $Matches[1] }
        if ($socket -and (Test-Path -LiteralPath $tend)) {
            Say "stopping tendd (pid $($p.ProcessId))"
            & $tend daemon stop --socket $socket 2>&1 | Out-Null
        }
    }
    $deadline = (Get-Date).AddSeconds(15)
    while (@(RunningFrom 'tendd' $dir).Count -gt 0 -and (Get-Date) -lt $deadline) {
        Start-Sleep -Milliseconds 300
    }
    foreach ($p in (RunningFrom 'tendd' $dir)) {
        Warn "tendd (pid $($p.ProcessId)) did not stop; ending it"
        Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue
    }
}

# --- registrations ------------------------------------------------------------
function TaskFor([string]$dir) {
    $task = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
    if (-not $task) { return @{ Exists = $false; Ours = $false } }
    $arguments = ($task.Actions | ForEach-Object { $_.Arguments }) -join ' '
    return @{ Exists = $true; Ours = (Mentions $arguments (Join-Path $dir 'tend.exe')); Arguments = $arguments }
}

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
    Register-ScheduledTask -TaskName $TaskName -Description "Start tendd at logon ($tend; installed by $ScriptName)" `
        -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
    Say "registered the logon task $TaskName"
}

function RunEntry {
    $item = Get-ItemProperty -LiteralPath $RunKey -Name $RunValue -ErrorAction SilentlyContinue
    if ($item) { return [string]$item.$RunValue }
    return $null
}

function ShortcutTarget {
    if (-not (Test-Path -LiteralPath $ShortcutPath)) { return $null }
    return (New-Object -ComObject WScript.Shell).CreateShortcut($ShortcutPath).TargetPath
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

# --- download and verify ------------------------------------------------------
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

# The release publishes tend-<tag>-<target>.zip and, beside it,
# tend-<tag>-<target>.sha256 (the archive name without its extension).
function FetchArchive([string]$work) {
    if ($Archive) {
        $zip = (Resolve-Path -LiteralPath $Archive).Path
        $sidecar = [IO.Path]::ChangeExtension($zip, '.sha256')
        if (-not (Test-Path -LiteralPath $sidecar)) { throw "$sidecar is missing; it must sit next to the zip" }
        return @{ Zip = $zip; Sidecar = $sidecar; Tag = $null }
    }
    $tag = ResolveTag $Version
    $base = "tend-$tag-$Target"
    $url = "https://github.com/$Repo/releases/download/$tag"
    $zip = Join-Path $work "$base.zip"
    Say "downloading $base.zip"
    Invoke-WebRequest -UseBasicParsing -Uri "$url/$base.zip" -OutFile $zip
    Invoke-WebRequest -UseBasicParsing -Uri "$url/$base.sha256" -OutFile (Join-Path $work "$base.sha256")
    return @{ Zip = $zip; Sidecar = (Join-Path $work "$base.sha256"); Tag = $tag }
}

# --- install / uninstall ------------------------------------------------------
function Install([string]$dir) {
    if (IsElevated) {
        Warn 'this PowerShell is elevated. Tend installs for your user only; tend-app started from here runs elevated. Prefer a normal PowerShell.'
    }
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        Warn "this is a $($env:PROCESSOR_ARCHITECTURE) machine; the x64 build runs under emulation"
    }

    # Ownership first, before anything is downloaded or changed.
    $manifest = $null
    if (Test-Path -LiteralPath $dir) {
        $manifest = ReadManifest $dir
        if (-not $manifest -and @(Get-ChildItem -LiteralPath $dir -Force).Count -gt 0) {
            throw "$dir is not empty and was not created by $ScriptName; choose a new or empty folder with -InstallDir. Nothing was changed."
        }
    }
    $task = TaskFor $dir
    if ($task.Exists -and -not $task.Ours) {
        throw "the logon task $TaskName belongs to another Tend install ($($task.Arguments)); run that install's $ScriptName -Uninstall first. Nothing was changed."
    }
    # An upgrade keeps the choices recorded at install unless given again.
    $withApp = -not $NoApp
    if ($manifest -and -not $Bound.ContainsKey('NoApp')) { $withApp = [bool]$manifest.app }
    $withShortcut = $withApp -and -not $NoShortcut
    if ($manifest -and -not $Bound.ContainsKey('NoShortcut')) { $withShortcut = $withApp -and [bool]$manifest.shortcut }
    $app = Join-Path $dir 'tend-app.exe'
    $shortcutOwned = $false
    if ($withShortcut) {
        $target = ShortcutTarget
        if ($target -and -not (SamePath $target $app)) {
            Warn "the Start menu entry $ShortcutPath opens $target; leaving it as it is"
        } else {
            $shortcutOwned = $true
        }
    }
    $run = RunEntry
    $runForeign = $withApp -and $run -and -not (Mentions $run $app)
    if ($runForeign) {
        Warn "another tend-app starts with Windows ($run); leaving that entry, so this install's app will not start with Windows"
    }

    $wanted = @('tend.exe', 'tendd.exe')
    if ($withApp) { $wanted += 'tend-app.exe' }
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
        $files = @{}
        foreach ($name in $wanted + @('install-windows.ps1')) {
            $found = Get-ChildItem -LiteralPath $unpacked -Recurse -File -Filter $name | Select-Object -First 1
            if ($found) { $files[$name] = $found.FullName }
        }
        foreach ($name in $wanted) {
            if (-not $files.ContainsKey($name)) { throw "$name is not in the archive. Nothing was installed." }
        }

        New-Item -ItemType Directory -Force -Path $dir | Out-Null
        StopTend $dir
        foreach ($name in $wanted) {
            $dest = Join-Path $dir $name
            $aside = "$dest.old"
            Remove-Item -LiteralPath $aside -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $dest) {
                # Windows refuses to overwrite a running executable but lets
                # one be renamed.
                try { Remove-Item -LiteralPath $dest -Force } catch { Move-Item -LiteralPath $dest -Destination $aside -Force }
            }
            Copy-Item -LiteralPath $files[$name] -Destination $dest
            Unblock-File -LiteralPath $dest -ErrorAction SilentlyContinue
        }
        # The installed copy of this script comes from the same release.
        $self = $files['install-windows.ps1']
        if (-not $self -and $PSCommandPath) { $self = $PSCommandPath }
        $installedSelf = Join-Path $dir $ScriptName
        if ($self -and -not (SamePath $self $installedSelf)) {
            Copy-Item -LiteralPath $self -Destination $installedSelf -Force
        }
        # A previous install with the app keeps its app file listed until an
        # uninstall removes it.
        $owned = @($wanted + @($ScriptName))
        if ($manifest) { $owned = @($owned + @($manifest.files) | Select-Object -Unique) }
        WriteManifest $dir ([ordered]@{
            version  = 1
            sid      = $Sid
            dir      = $dir
            release  = $fetched.Tag
            files    = $owned
            task     = $TaskName
            shortcut = $(if ($shortcutOwned) { $ShortcutPath } elseif ($manifest) { $manifest.shortcut } else { $null })
            app      = $withApp
        })
        Say "installed $($wanted -join ', ') into $dir"
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    AddToPath $dir
    WarnShadowed $dir
    $env:Path = "$dir;$env:Path"
    $tend = Join-Path $dir 'tend.exe'
    $installed = (& $tend --version) 2>$null
    if ($installed) { Say "$installed" }

    if (-not (Get-Command rmux -ErrorAction SilentlyContinue)) {
        Warn 'rmux is not on PATH. tendd runs without it, but sees no panes until it is installed: winget install Helvesec.RMUX'
    }

    RegisterDaemonTask $tend
    if ($withApp) {
        # The app needs the loopback dashboard with a token; this keeps an
        # existing [dashboard] section as it is.
        & $tend init --component dashboard --yes --start-daemon=false
        if ($LASTEXITCODE -ne 0) { Warn "tend init --component dashboard exited with $LASTEXITCODE" }
        if ($shortcutOwned) {
            $link = (New-Object -ComObject WScript.Shell).CreateShortcut($ShortcutPath)
            $link.TargetPath = $app
            $link.WorkingDirectory = $dir
            $link.Description = 'Tend'
            $link.Save()
            Say "added the Start menu shortcut $ShortcutPath"
        }
    }
    if ($NoStart) { Say 'done (not started: -NoStart)'; return }
    StartDaemon $dir
    if ($withApp) {
        if ($runForeign) {
            Start-Process -FilePath $app -WorkingDirectory $dir
        } else {
            # Registers toasts and the start-with-Windows entry, then runs it.
            Start-Process -FilePath $app -ArgumentList '--install' -WorkingDirectory $dir
        }
        Say 'started tend-app; it is in the notification area (Windows may put it under the ^ arrow)'
    }
    Say 'done. In a new terminal, run tend init (inside an rmux pane) to wire agent hooks and key bindings.'
}

function Remove([string]$dir) {
    $manifest = $null
    if (Test-Path -LiteralPath $dir) { $manifest = ReadManifest $dir }
    if (-not $manifest) {
        throw "no $ManifestName in $dir, so it is not an install by $ScriptName; nothing was removed. Pass -InstallDir for another folder."
    }
    $app = Join-Path $dir 'tend-app.exe'
    if ((Test-Path -LiteralPath $app) -and (Mentions (RunEntry) $app)) {
        # Removes the start-with-Windows entry and the toast registration.
        $p = Start-Process -FilePath $app -ArgumentList '--uninstall' -PassThru -Wait
        if ($p.ExitCode -ne 0) { Warn "tend-app --uninstall exited with $($p.ExitCode)" }
    }
    StopTend $dir
    $task = TaskFor $dir
    if ($task.Ours) {
        Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
        Say "removed the logon task $TaskName"
    }
    if ($manifest.shortcut -and (SamePath ([string](ShortcutTarget)) $app)) {
        Remove-Item -LiteralPath $manifest.shortcut -Force
        Say "removed the Start menu shortcut"
    }
    RemoveFromPath $dir
    foreach ($name in @($manifest.files) + @($ManifestName)) {
        foreach ($path in @((Join-Path $dir $name), (Join-Path $dir "$name.old"))) {
            Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        }
    }
    if (@(Get-ChildItem -LiteralPath $dir -Force).Count -eq 0) {
        Remove-Item -LiteralPath $dir -Force
        Say "removed $dir"
    } else {
        Say "left $dir in place: it holds files this script did not install"
    }
    Say 'uninstalled. Kept your settings and data:'
    foreach ($data in @((Join-Path $env:APPDATA 'tend'), (Join-Path $env:LOCALAPPDATA 'tend'))) {
        if (Test-Path -LiteralPath $data) { Say "  $data" }
    }
    Say 'tend init --uninstall (run before this) also removes the hooks and rmux bindings tend init added.'
}

[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$dir = ResolveInstallDir
if ($Uninstall) { Remove $dir } else { Install $dir }
