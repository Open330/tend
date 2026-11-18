# Fail when a function in a PowerShell script assigns (or loops over) a
# variable with the name of one the script itself sets or takes as a
# parameter. PowerShell names ignore case and are dynamically scoped: a
# function's local $target hides the script's $Target from every function it
# calls. install-tend.ps1 v0.10.24 shipped that way: the default online
# install asked for tend-v0.10.24-.zip.
#
# What counts: the script's names are its parameters and every variable it
# assigns outside a function ($x or $script:x). A function collides when it
# assigns or loops over one of those names unqualified or as $local:x; a
# write to $script:x, $global:x, $private:x (invisible to callees) or a
# provider path ($env:x) does not. Function *parameters* are not analysed:
# a parameter that shares a script name is a deliberate input and needs a
# reviewer's eye, not this check. scripts/check-ps-scope.tests.ps1 holds the
# cases.
#
#   pwsh -NoProfile -File scripts/check-ps-scope.ps1 scripts/install-windows.ps1
param([Parameter(Mandatory = $true)][string[]]$Path)
$ErrorActionPreference = 'Stop'
$problems = New-Object System.Collections.Generic.List[string]

# The name without a scope qualifier, lower-cased: $script:Target -> target.
function Bare($variablePath) {
    ($variablePath.UserPath -replace '^(script|local|private|global|using):', '').ToLowerInvariant()
}

function NameOf($expression) {
    # $x = ..., [type]$x = ...
    while ($expression -is [System.Management.Automation.Language.ConvertExpressionAst] -or
        $expression -is [System.Management.Automation.Language.AttributedExpressionAst]) {
        $expression = $expression.Child
    }
    if ($expression -is [System.Management.Automation.Language.VariableExpressionAst]) { return $expression.VariablePath }
    return $null
}

foreach ($file in $Path) {
    $tokens = $null; $errors = $null
    $ast = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $file).Path, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { $problems.Add("${file}: does not parse: $($errors[0].Message)"); continue }

    $scriptNames = @{}
    if ($ast.ParamBlock) {
        foreach ($p in $ast.ParamBlock.Parameters) { $scriptNames[(Bare $p.Name.VariablePath)] = $true }
    }
    # Assignments outside any function body are the script's own.
    $outside = $ast.FindAll({
            param($node)
            $node -is [System.Management.Automation.Language.AssignmentStatementAst]
        }, $true) | Where-Object {
        $parent = $_.Parent
        while ($parent -and $parent -isnot [System.Management.Automation.Language.FunctionDefinitionAst]) { $parent = $parent.Parent }
        -not $parent
    }
    foreach ($assignment in $outside) {
        $name = NameOf $assignment.Left
        if ($name -and -not $name.IsDriveQualified) { $scriptNames[(Bare $name)] = $true }
    }

    $functions = $ast.FindAll({ param($node) $node -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true)
    foreach ($function in $functions) {
        $locals = @()
        $locals += @($function.Body.FindAll({ param($node) $node -is [System.Management.Automation.Language.AssignmentStatementAst] }, $true) |
            ForEach-Object { NameOf $_.Left })
        $locals += @($function.Body.FindAll({ param($node) $node -is [System.Management.Automation.Language.ForEachStatementAst] }, $true) |
            ForEach-Object { $_.Variable.VariablePath })
        foreach ($local in $locals) {
            if (-not $local -or $local.IsScript -or $local.IsGlobal -or $local.IsPrivate -or $local.IsDriveQualified) { continue }
            $key = Bare $local
            if ($scriptNames.ContainsKey($key)) {
                $problems.Add("${file}: function $($function.Name) sets `$$($local.UserPath), which hides the script's own `$$(Bare $local) from what it calls; rename it")
            }
        }
    }
}

if ($problems.Count) {
    $problems | Sort-Object -Unique | ForEach-Object { Write-Output "::error::$_" }
    exit 1
}
Write-Output "ok: no function assigns or loops over a script-level name in $($Path -join ', ') (function parameters are not checked)"
