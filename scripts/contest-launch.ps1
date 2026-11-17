#requires -Version 7.4
param([string]$Command)
$CommandArguments = $args
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
try {
    if ($Command -notin @('compilecpp', 'submit')) { throw '请选择 compilecpp 或 submit。' }
    # Read the activation pointer once. Every dependency of this invocation uses that version.
    $active = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'active.json') -Raw | ConvertFrom-Json
    $manifestPath = Join-Path $active.directory 'manifest.json'
    if ((Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash -ine $active.manifest_sha256) {
        throw '脚本清单校验失败，请联系工作人员重新部署。'
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    foreach ($file in $manifest.files) {
        if ((Get-FileHash -LiteralPath (Join-Path $active.directory $file.path) -Algorithm SHA256).Hash -ine $file.sha256) {
            throw '脚本文件校验失败，请联系工作人员重新部署。'
        }
    }
    $configuration = Join-Path $active.directory 'contest.json'
    if ((Get-FileHash -LiteralPath $configuration -Algorithm SHA256).Hash -ine $active.configuration_sha256) {
        throw '比赛配置校验失败，请联系工作人员重新部署。'
    }
    & (Join-Path $active.directory ($Command + '.ps1')) -Configuration $configuration @CommandArguments
    exit $LASTEXITCODE
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
}
