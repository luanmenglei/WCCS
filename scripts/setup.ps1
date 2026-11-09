#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Get-Command git -ErrorAction Stop | Out-Null
Get-Command python -ErrorAction Stop | Out-Null
& python -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)'
if ($LASTEXITCODE -ne 0) {
    throw 'Python 3.11 or newer is required.'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$gitRoot = & git -C $repositoryRoot rev-parse --show-toplevel
if ($LASTEXITCODE -ne 0) {
    throw 'Run setup from an initialized WCCS Git repository.'
}
if ([IO.Path]::GetFullPath($gitRoot) -ne [IO.Path]::GetFullPath($repositoryRoot)) {
    throw 'The scripts directory must belong to the WCCS repository root.'
}

$settings = @{
    'core.autocrlf' = 'false'
    'core.eol' = 'lf'
    'core.hooksPath' = '.githooks'
}
foreach ($setting in $settings.GetEnumerator()) {
    & git -C $repositoryRoot config --local $setting.Key $setting.Value
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to configure $($setting.Key)."
    }
}

& python (Join-Path $PSScriptRoot 'check_repo.py')
if ($LASTEXITCODE -ne 0) {
    throw 'Repository checks failed.'
}
Write-Output 'WCCS local Git configuration and pre-commit hook are ready.'
