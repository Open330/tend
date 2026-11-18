# Cases for scripts/check-ps-scope.ps1: each fixture must pass or fail as
# marked. Run in CI next to the check itself.
#
#   pwsh -NoProfile -File scripts/check-ps-scope.tests.ps1
$ErrorActionPreference = 'Stop'
$check = Join-Path $PSScriptRoot 'check-ps-scope.ps1'
$cases = [ordered]@{
    # The v0.10.24 bug: a bare local with another case.
    'bare local hides a script name'          = @{ Fail = $true; Code = '$Target = "good"; function F { $target = "bad"; G }; function G { $Target }' }
    'script-qualified script name'           = @{ Fail = $true; Code = '$script:Target = "good"; function F { $target = "bad" }' }
    'explicit local hides a script name'      = @{ Fail = $true; Code = '$Target = "good"; function F { $local:target = "bad" }' }
    'typed local hides a script name'         = @{ Fail = $true; Code = '$Target = "good"; function F { [string]$target = "bad" }' }
    'foreach variable hides a script name'    = @{ Fail = $true; Code = '$Target = "good"; function F { foreach ($target in 1, 2) { } }' }
    'local hides a script parameter'          = @{ Fail = $true; Code = 'param([string]$Version)|function F { $version = "x" }' }
    'write to the script scope is fine'       = @{ Fail = $false; Code = '$Target = "good"; function F { $script:Target = "new" }' }
    'environment variable is fine'            = @{ Fail = $false; Code = '$Path = "x"; function F { $env:Path = "y" }' }
    'private local is invisible to callees'   = @{ Fail = $false; Code = '$Target = "good"; function F { $private:target = "bad" }' }
    'unrelated local is fine'                 = @{ Fail = $false; Code = '$Target = "good"; function F { $linkTarget = "x" }' }
    # Documented blind spot: parameters are inputs, reviewed by people.
    'parameter is not analysed'               = @{ Fail = $false; Code = '$Target = "good"; function F($target) { }' }
}
$dir = Join-Path ([IO.Path]::GetTempPath()) ("check-ps-scope-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dir | Out-Null
$wrong = 0
try {
    foreach ($name in $cases.Keys) {
        $file = Join-Path $dir 'case.ps1'
        # '|' separates lines (a param block must stand on its own).
        Set-Content -LiteralPath $file -Value ($cases[$name].Code -replace '\|', "`n")
        & pwsh -NoProfile -File $check $file *> $null
        $failed = $LASTEXITCODE -ne 0
        if ($failed -eq $cases[$name].Fail) { Write-Output "ok   $name" } else { Write-Output "FAIL $name (exit $LASTEXITCODE)"; $wrong++ }
    }
} finally {
    Remove-Item -LiteralPath $dir -Recurse -Force -ErrorAction SilentlyContinue
}
if ($wrong) { exit 1 }
