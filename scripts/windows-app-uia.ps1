# Drives Tend for Windows' Home window through Windows UI Automation, the
# accessibility tree WebView2 exposes, as a person would with the mouse:
# answers every question with its "yes" option (an option button, then
# "Send") and accepts every result ("Accept"). Used by windows-app-e2e.ps1; runs in Windows
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

# Home follows the system language: each label is given in every language
# the page ships (English, Korean).
$Send = @("Send", "보내기")
$Accept = @("Accept", "수락")

function Buttons($win, [string[]]$names) {
    $isButton = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    @($win.FindAll($Scope::Descendants, $isButton) | Where-Object { $names -contains $_.Current.Name })
}

# A question's option button reads its number and its text ("1 yes").
function OptionButtons($win, [string]$option) {
    $isButton = New-Object System.Windows.Automation.PropertyCondition($AE::ControlTypeProperty, [System.Windows.Automation.ControlType]::Button)
    @($win.FindAll($Scope::Descendants, $isButton) | Where-Object { $_.Current.Name -match "^\d+\s*$([regex]::Escape($option))$" })
}

function Press($element) {
    $element.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
}

$win = Window
Write-Output "window: $($win.Current.Name)"
# Wait for the page to render its cards.
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline -and (OptionButtons $win "yes").Count -eq 0 -and (Buttons $win $Accept).Count -eq 0) { Start-Sleep -Milliseconds 500 }

# The page re-renders when it reads the queue again (0.8 s after a decision,
# and on its own timer), which can replace a button between finding and
# pressing it: such an attempt is retried with freshly found buttons.
function Gone($err) { "$($err.Exception)" -match "ElementNotAvailable" }

$answered = 0
for ($i = 0; $i -lt 20; $i++) {
    $yes = OptionButtons $win "yes"
    if ($yes.Count -eq 0) { break }
    try {
        # The option opens the card's answer field filled with it; Send sends.
        Press $yes[0]
        # A press that lands on a card the page has just replaced opens
        # nothing: wait for the field, else find the option again.
        $send = @()
        $until = (Get-Date).AddSeconds(4)
        while ($send.Count -eq 0 -and (Get-Date) -lt $until) { Start-Sleep -Milliseconds 300; $send = Buttons $win $Send }
        if ($send.Count -eq 0) {
            Write-Output "no answer field after choosing an option; retrying"
            Start-Sleep -Seconds 1
            continue
        }
        Press $send[0]
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
    $good = Buttons $win $Accept
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
