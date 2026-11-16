# End-to-end test of Tend for Windows (crates/tend-app) on a real desktop.
#
# Two scratch tendd daemons, each watching its own private rmux server
# (TEND_RMUX_SOCKET): "local" (the controller, with the dashboard the app
# talks to) and "win-b", a Fleet host reached over `ssh localhost`. Agents in
# both ask a question and file a result. The app is installed for start with
# Windows under a test name, started by its Run command, and must:
#   - raise toasts (one summary at start, then one for a new question);
#   - open Home on a card when asked by a second instance;
#   - let the person answer and decide in Home (driven through Windows UI
#     Automation: the WebView2 accessibility tree, no debugging port);
# and both daemons must then hold the answers and decisions.
#
# Run it in the interactive session of the person logged on (toasts and UI
# Automation need a desktop). Over ssh, `-Desktop` hands the run to a
# one-shot scheduled task in that session and waits for it.
#
# Requires: tend.exe, tendd.exe and tend-app.exe in -Bin; rmux on PATH;
# `ssh localhost` with key authentication (BatchMode); WebView2 Runtime.
# Touches nothing of the person's: scratch XDG directories, private rmux
# labels, a test AppUserModelID and Run value (removed at the end).
#
#   pwsh -NoProfile -File scripts/windows-app-e2e.ps1 -Bin target\release [-Desktop]

param(
    [Parameter(Mandatory = $true)][string]$Bin,
    [string]$Root = (Join-Path $HOME "tend-app-e2e"),
    [switch]$Desktop,
    [switch]$Keep
)

$ErrorActionPreference = "Stop"
$PSStyle.OutputRendering = "PlainText"
$Bin = (Resolve-Path $Bin).Path

if ($Desktop) {
    # Hand the run to the logged-on desktop and wait for its result file.
    $task = "tend-app-e2e"
    $result = Join-Path $Root "result.txt"
    New-Item -ItemType Directory -Force $Root | Out-Null
    Remove-Item $result -ErrorAction SilentlyContinue
    $pwsh = (Get-Command pwsh).Source
    $args = "-NoProfile -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Bin `"$Bin`" -Root `"$Root`"" + $(if ($Keep) { " -Keep" } else { "" })
    schtasks /Create /F /TN $task /SC ONCE /ST 23:59 /IT /RL LIMITED /TR "`"$pwsh`" $args" | Out-Null
    schtasks /Run /TN $task | Out-Null
    $deadline = (Get-Date).AddMinutes(10)
    while (-not (Test-Path $result) -and (Get-Date) -lt $deadline) { Start-Sleep 2 }
    schtasks /Delete /F /TN $task | Out-Null
    if (-not (Test-Path $result)) { Write-Output "FAIL: no result within 10 minutes"; exit 1 }
    Get-Content (Join-Path $Root "e2e.log")
    $verdict = Get-Content $result
    Write-Output $verdict
    if ($verdict -like "PASS*") { exit 0 } else { exit 1 }
}

# ---------------------------------------------------------------- helpers

New-Item -ItemType Directory -Force $Root | Out-Null
$Log = Join-Path $Root "e2e.log"
Set-Content $Log ""
function Say([string]$text) { $line = "$(Get-Date -Format HH:mm:ss) $text"; Add-Content $Log $line; Write-Output $line }
$failures = New-Object System.Collections.Generic.List[string]
function Check([bool]$ok, [string]$what) {
    if ($ok) { Say "ok   $what" } else { Say "FAIL $what"; $failures.Add($what) }
}
function WaitFor([scriptblock]$cond, [int]$seconds, [string]$what) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -lt $deadline) {
        try { if (& $cond) { return $true } } catch { }
        Start-Sleep -Milliseconds 500
    }
    Say "timed out waiting for: $what" | Out-Null
    return $false
}
function RandomHex([int]$bytes) {
    $buf = [byte[]]::new($bytes)
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($buf)
    -join ($buf | ForEach-Object { $_.ToString("x2") })
}

$rmux = (Get-Command rmux).Source
$tend = Join-Path $Bin "tend.exe"
$tendd = Join-Path $Bin "tendd.exe"
$app = Join-Path $Bin "tend-app.exe"
foreach ($exe in @($tend, $tendd, $app)) { if (-not (Test-Path $exe)) { throw "missing $exe" } }
Remove-Item Env:RMUX, Env:RMUX_PANE, Env:TMUX, Env:TMUX_PANE -ErrorAction SilentlyContinue

$token = RandomHex 32
$port = Get-Random -Minimum 20000 -Maximum 40000
$appId = "dev.tend.windows.e2e"
$runValue = "Tend-e2e"
$appState = Join-Path $Root "app-state"
$label = @{ local = "tend-app-e2e-a"; "win-b" = "tend-app-e2e-b" }
$started = New-Object System.Collections.Generic.List[int]

function NodeEnv([string]$name) {
    $dir = Join-Path $Root $name
    @{
        XDG_CONFIG_HOME = "$dir\config"; XDG_DATA_HOME = "$dir\data"
        XDG_STATE_HOME = "$dir\state"; XDG_RUNTIME_DIR = "$dir\run"
    }
}
function EnvLines([hashtable]$envs) { ($envs.GetEnumerator() | ForEach-Object { "set $($_.Key)=$($_.Value)" }) -join "`r`n" }

# A detached process (WMI: no inherited handles, no job), so it outlives
# this script's own children and the rmux servers.
function Detached([string]$commandLine) {
    $r = Invoke-CimMethod Win32_Process -MethodName Create -Arguments @{ CommandLine = $commandLine }
    if ($r.ReturnValue -ne 0) { throw "could not start: $commandLine" }
    $started.Add([int]$r.ProcessId)
    [int]$r.ProcessId
}

function Api([string]$path) {
    Invoke-RestMethod -Uri "http://127.0.0.1:$port$path" -Headers @{ Authorization = "Bearer $token" } -TimeoutSec 10
}

function Cleanup {
    Say "cleanup"
    Get-CimInstance Win32_Process -Filter "Name='tend-app.exe'" | Where-Object { $_.ExecutablePath -eq $app } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    Get-CimInstance Win32_Process -Filter "Name='tendd.exe'" | Where-Object { $_.CommandLine -like "*$Root*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
    foreach ($l in $label.Values) {
        # Always an explicit -L on a private label: never the person's server.
        & $rmux -L $l kill-server 2>$null | Out-Null
    }
    $env:TEND_APP_ID = $appId; $env:TEND_APP_RUN_VALUE = $runValue; $env:TEND_APP_STATE_DIR = $appState
    & $app --uninstall | Out-Null
}

try {
    # ---------------------------------------------------- two scratch daemons
    Get-ChildItem $Root -Exclude "e2e.log", "result.txt" -ErrorAction SilentlyContinue | Remove-Item -Recurse -Force
    $sockets = @{}
    foreach ($name in @("local", "win-b")) {
        $envs = NodeEnv $name
        foreach ($d in $envs.Values) { New-Item -ItemType Directory -Force "$d\tend" | Out-Null }
        Detached "`"$rmux`" -L $($label[$name]) new-session -d -s hold `"pwsh -NoProfile -Command Start-Sleep 3600`"" | Out-Null
        Start-Sleep 2
        $sockets[$name] = (& $rmux -L $label[$name] display-message -p -t hold '#{socket_path}').Trim()
        Say "$name rmux server $($sockets[$name])"
    }

    $b = NodeEnv "win-b"
    $bDir = Join-Path $Root "win-b"
    # The Fleet relay runs through ssh in the login shell, with the person's
    # environment: a wrapper gives it win-b's scratch directories.
    Set-Content "$bDir\tend-b.cmd" -Encoding ascii -Value "@echo off`r`n$(EnvLines $b)`r`n`"$tend`" %*"
    $relPath = [System.IO.Path]::GetRelativePath($HOME, "$bDir\tend-b.cmd").Replace('\', '/')

    $aConfig = @"
[dashboard]
enabled = true
bind = "127.0.0.1:$port"
token = "$token"

[collaboration]
enabled = true

[fleet]
enabled = true

[fleet.hosts.win-b]
ssh = "localhost"
tend_path = "$relPath"
remote_socket = '$bDir\run\tend.sock'
mode = "control"
"@
    Set-Content "$Root\local\config\tend\config.toml" -Value $aConfig
    Set-Content "$Root\win-b\config\tend\config.toml" -Value "[collaboration]`nenabled = true`n"

    foreach ($name in @("local", "win-b")) {
        $envs = NodeEnv $name
        $dir = Join-Path $Root $name
        Set-Content "$dir\tendd.cmd" -Encoding ascii -Value @"
@echo off
$(EnvLines $envs)
set TEND_HOSTS=rmux
set TEND_RMUX_SOCKET=$($sockets[$name])
"$tendd" --socket "$dir\run\tend.sock" --log-file "$dir\tendd.log"
"@
        Detached "cmd.exe /c `"$dir\tendd.cmd`"" | Out-Null
    }
    Check (WaitFor { (Api "/api/health").ok } 30 "local dashboard") "the local dashboard answers"
    Check (WaitFor { ((Api "/api/fleet").hosts | Where-Object { $_.alias -eq "win-b" }).state -eq "online" } 60 "win-b online") "the Fleet host win-b is online over ssh"

    # ---------------------------------------------------------- seed agents
    function Seed([string]$name, [string]$session, [string]$question, [string]$summary) {
        $envs = NodeEnv $name
        $dir = Join-Path $Root $name
        $mcp = @(
            '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"e2e","version":"0"}}}',
            '{"jsonrpc":"2.0","method":"notifications/initialized"}',
            ('{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tend_request_attention","arguments":{"body":"' + $question + '","options":["yes","no"],"recommendation":"yes"}}}')
        ) -join "`n"
        Set-Content "$dir\$session.mcp" -Value $mcp
        $hook = '{"session_id":"' + $session + '","cwd":"' + $dir.Replace('\', '\\') + '","prompt":"work on ' + $session + '"}'
        Set-Content "$dir\$session.hook" -Value $hook
        Set-Content "$dir\$session.ps1" -Value @"
$(($envs.GetEnumerator() | ForEach-Object { "`$env:$($_.Key) = '$($_.Value)'" }) -join "`n")
Get-Content '$dir\$session.hook' -Raw | & '$tend' hook claude --event user_prompt_submit
Start-Sleep 2
Get-Content '$dir\$session.mcp' -Raw | & '$tend' mcp *> '$dir\$session.out'
& '$tend' review request --summary '$summary' --session $session --provider claude *>> '$dir\$session.out'
Start-Sleep 3600
"@
        Detached "`"$rmux`" -L $($label[$name]) new-window -t hold `"pwsh -NoProfile -File $dir\$session.ps1`"" | Out-Null
    }
    Seed "local" "e2e-asker-a" "Ship the Windows app?" "Windows app phase 1 result"
    Seed "win-b" "e2e-asker-b" "Merge the Fleet change?" "Fleet host result"
    function QueueItems { @((Api "/api/decisions").hosts | ForEach-Object { $_.queue.items }) | Where-Object { $_.kind -in @("human_request", "review") } }
    Check (WaitFor { (QueueItems).Count -ge 4 } 60 "4 seeded items") "the queue holds 2 questions and 2 results across local and win-b"

    # --------------------------------------------------- install and start
    $env:XDG_CONFIG_HOME = "$Root\local\config"; $env:XDG_DATA_HOME = "$Root\local\data"
    $env:XDG_STATE_HOME = "$Root\local\state"; $env:XDG_RUNTIME_DIR = "$Root\local\run"
    $env:TEND_APP_ID = $appId; $env:TEND_APP_RUN_VALUE = $runValue; $env:TEND_APP_STATE_DIR = $appState
    $appLog = Join-Path $appState "app.log"
    Start-Process -FilePath $app -ArgumentList "--install"
    Check (WaitFor { (Get-Content $appLog -Raw) -match "installed" } 20 "install") "tend-app --install registered itself"
    $run = (Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run" -Name $runValue -ErrorAction SilentlyContinue).$runValue
    Check ($run -eq "`"$app`" --background") "the Run value starts the app in the tray: $run"
    Check ((Get-ItemProperty "HKCU:\Software\Classes\AppUserModelId\$appId" -ErrorAction SilentlyContinue).DisplayName -eq "Tend") "toasts are registered as Tend"
    # Start it the way logon does: the Run value's command line, nothing else.
    Get-CimInstance Win32_Process -Filter "Name='tend-app.exe'" | Where-Object { $_.ExecutablePath -eq $app } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    Start-Sleep 1
    Start-Process -FilePath "cmd.exe" -ArgumentList "/c", "start `"`" $run" -WindowStyle Hidden
    Check (WaitFor { (Get-Process tend-app -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $app }) } 10 "app process") "the Run command starts tend-app"
    Check (WaitFor { (Get-Content $appLog -Raw) -match "toast summary:4" } 40 "summary toast") "a summary toast for the 4 waiting items"

    # A new question after start is its own toast.
    Seed "local" "e2e-asker-c" "Also ship the docs?" "Docs result"
    Check (WaitFor { (Get-Content $appLog -Raw) -match "toast local\|q\|" } 60 "single toast") "a toast for the new question"

    # ------------------------------------------- Home on a card, answer, decide
    $first = (QueueItems | Where-Object { $_.kind -eq "human_request" } | Select-Object -First 1).request_id
    & $app --open $first
    Check (WaitFor { (Get-Content $appLog -Raw) -match "home opened $first" } 20 "home opened") "a second instance hands 'open Home on $first' to the running app"

    $ui = Join-Path $PSScriptRoot "windows-app-uia.ps1"
    $driven = & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $ui -ProcessId ((Get-Process tend-app | Where-Object { $_.Path -eq $app } | Select-Object -First 1).Id) 2>&1
    $driven | ForEach-Object { Say "uia: $_" }

    function DecisionHistory { @((Api "/api/decisions/history").hosts | ForEach-Object { [pscustomobject]@{ alias = $_.alias; items = @($_.history.items) } }) }
    $answeredLocal = WaitFor { ((DecisionHistory | Where-Object alias -eq "local").items | Where-Object { $_.kind -eq "human_answer" -and $_.answer -eq "yes" }).Count -ge 2 } 30 "local answers"
    Check $answeredLocal "both local questions answered 'yes' from Home"
    Check (WaitFor { ((DecisionHistory | Where-Object alias -eq "win-b").items | Where-Object { $_.kind -eq "human_answer" -and $_.answer -eq "yes" }).Count -ge 1 } 30 "remote answer") "the win-b question answered over Fleet from Home"
    if (-not $answeredLocal) { (Api "/api/decisions/history") | ConvertTo-Json -Depth 12 | Set-Content (Join-Path $Root "history.json") }
    Check (WaitFor { ((DecisionHistory | Where-Object alias -eq "local").items | Where-Object { $_.kind -eq "review" -and $_.disposition.kind -eq "accepted" }).Count -ge 2 } 30 "local reviews") "both local results accepted from Home"
    Check (WaitFor { ((DecisionHistory | Where-Object alias -eq "win-b").items | Where-Object { $_.kind -eq "review" -and $_.disposition.kind -eq "accepted" }).Count -ge 1 } 30 "remote review") "the win-b result accepted over Fleet from Home"
    Check ((QueueItems).Count -eq 0) "nothing is left in the queue"
    $audit = Get-ChildItem "$Root\local\data" -Recurse -Filter "dashboard-decisions.jsonl" | Select-Object -First 1
    Check ($audit -and ((Get-Content $audit.FullName) | Where-Object { $_ -match '"web:token"' }).Count -ge 6) "every decision is audited as web:token"
    Check (WaitFor { ((Get-Content $appLog) | Where-Object { $_ -match " withdrawn " }).Count -ge 1 } 40 "toasts withdrawn") "decided items take their toasts with them"
    Check (-not ((Get-Content $appLog -Raw) -match [regex]::Escape($token))) "the token is in no log"

    # ------------------------------------------------- an impostor on the port
    Get-CimInstance Win32_Process -Filter "Name='tendd.exe'" | Where-Object { $_.CommandLine -like "*$Root\local\*" } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
    Start-Sleep 1
    $impostor = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, $port)
    $impostor.Start()
    $captured = New-Object System.Text.StringBuilder
    $deadline = (Get-Date).AddSeconds(40)
    while ((Get-Date) -lt $deadline) {
        if ($impostor.Pending()) {
            $c = $impostor.AcceptTcpClient(); $s = $c.GetStream(); $s.ReadTimeout = 2000
            $buf = [byte[]]::new(8192)
            try { $n = $s.Read($buf, 0, $buf.Length); [void]$captured.Append([System.Text.Encoding]::ASCII.GetString($buf, 0, $n)) } catch { }
            $reply = [System.Text.Encoding]::ASCII.GetBytes("HTTP/1.1 200 OK`r`nContent-Length: 79`r`n`r`n{""version"":1,""proof"":""0000000000000000000000000000000000000000000000000000000000000000""}")
            try { $s.Write($reply, 0, $reply.Length) } catch { }
            Start-Sleep -Milliseconds 300
            $c.Close()
        } else { Start-Sleep -Milliseconds 200 }
    }
    $impostor.Stop()
    $text = $captured.ToString()
    Check ($text -match "/api/proof\?nonce=") "the app asked the impostor for a proof"
    Check (-not ($text -match "Authorization") -and -not ($text -match [regex]::Escape($token))) "the impostor never saw the token"
    Check ((Get-Content $appLog -Raw) -match "not tendd|not running") "the tray reports the dashboard address is not tendd"
}
catch {
    Say "error: $_"
    $failures.Add("script error: $_")
}
finally {
    if (-not $Keep) { Cleanup }
    $verdict = if ($failures.Count -eq 0) { "PASS" } else { "FAIL: " + ($failures -join "; ") }
    Say $verdict
    Set-Content (Join-Path $Root "result.txt") $verdict
}
