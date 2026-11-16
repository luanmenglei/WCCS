#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$ServerConfig)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$configuration = Get-Content -LiteralPath $ServerConfig -Raw | ConvertFrom-Json
$previousDatabase = $env:WCCS_TEST_DATABASE_URL
Push-Location $repositoryRoot
try {
    $env:WCCS_TEST_DATABASE_URL = $configuration.database_url
    & (Join-Path $PSScriptRoot 'check-rust.ps1')
    # Run the capacity case separately, without competing test workloads.
    & cargo test --locked -p wccs-server --target x86_64-pc-windows-msvc --test http -- --ignored --skip concurrent_control_preview_commit_and_reconnect
    if ($LASTEXITCODE -ne 0) { throw 'PostgreSQL integration check failed.' }
    & npm --prefix web ci
    if ($LASTEXITCODE -ne 0) { throw 'Frontend installation failed.' }
    & npm --prefix web run check
    if ($LASTEXITCODE -ne 0) { throw 'Frontend checks failed.' }
} finally {
    $env:WCCS_TEST_DATABASE_URL = $previousDatabase
    Pop-Location
}
