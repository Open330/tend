# Drives Tend for Windows' Home window through Windows UI Automation, the
# accessibility tree WebView2 exposes, as a person would with the mouse:
# answers every question with its first option ("yes") and accepts every
# result ("Looks good"). Used by windows-app-e2e.ps1; runs in Windows
# PowerShell 5.1, which ships the UI Automation client.
param([Parameter(Mandatory = $true)][int]$ProcessId)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$AE = [System.Windows.Automation.AutomationElement]
$Scope = [System.Windows.Automation.TreeScope]

function Window {
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        $byPid = New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $ProcessId)
        $win = $AE::RootElement.FindFirst($Scope::Children, $byPid)
        if ($win) { return $win }
        Start-Sleep -Milliseconds 500
    }
    throw "no Home window for process $ProcessId"
}

function Buttons($win, [string]$name) {
    $isButton = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    $named = New-Object System.Windows.Automation.PropertyCondition($AE::NameProperty, $name)
    $both = New-Object System.Windows.Automation.AndCondition($isButton, $named)
    @($win.FindAll($Scope::Descendants, $both))
}

function Press($element) {
    $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

$win = Window
Write-Output "window: $($win.Current.Name)"
# Wait for the page to render its cards.
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline -and (Buttons $win "Send answer").Count -eq 0) { Start-Sleep -Milliseconds 500 }

# The page re-renders when it reads the queue again (0.8 s after a decision,
# and on its own timer), which can replace a button between finding and
# pressing it: such an attempt is retried with freshly found buttons.
function Gone($err) { "$($err.Exception)" -match "ElementNotAvailable" }

$answered = 0
for ($i = 0; $i -lt 20; $i++) {
    $yes = Buttons $win "yes"
    $send = Buttons $win "Send answer"
    if ($yes.Count -eq 0 -or $send.Count -eq 0) { break }
    try {
        Press $yes[0]
        Start-Sleep -Milliseconds 400
        Press (Buttons $win "Send answer")[0]
    } catch {
        if (-not (Gone $_)) { throw }
        Write-Output "re-rendered while answering; retrying"
        Start-Sleep -Seconds 1
        continue
    }
    $answered++
    Write-Output "answered question $answered"
    Start-Sleep -Seconds 3
}

$accepted = 0
for ($i = 0; $i -lt 20; $i++) {
    $good = Buttons $win "Looks good"
    if ($good.Count -eq 0) { break }
    try { Press $good[0] } catch {
        if (-not (Gone $_)) { throw }
        Write-Output "re-rendered while accepting; retrying"
        Start-Sleep -Seconds 1
        continue
    }
    $accepted++
    Write-Output "accepted result $accepted"
    Start-Sleep -Seconds 3
}
Write-Output "done: $answered answered, $accepted accepted"
