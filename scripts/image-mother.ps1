#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][ValidateSet('Upgrade','Start','Recover','Seal','Diagnose')][string]$Action)
$ErrorActionPreference = 'Stop'
$bundle = $PSScriptRoot
$source = Join-Path $bundle 'image'
. (Join-Path $source 'package/scripts/image-common.ps1')
Assert-ImageSealed $source
$targetPath = Join-Path $bundle 'mother-target.json'
$target = Read-ClientJson $targetPath
$guid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
Assert-ClientTarget $target.machine_name $target.machine_guid (Get-ImageMachineName) $guid
$imageRoot = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'WCCS-Image'
$configuration = Get-ImageConfiguration (Join-Path $source 'image.json')
if ($target.install_directory -ne $configuration.install_directory -or $target.service_name -ne $configuration.service_name -or
    $target.server -ne $configuration.server -or $target.ca_sha256 -ne $configuration.ca_sha256) { throw 'mother_configuration_mismatch' }
$runtime = Join-Path $source 'package/runtime/pwsh.exe'
$field = Join-Path $source 'package/scripts/client-field.ps1'
if ($Action -eq 'Upgrade') {
    Assert-ClientProtected (Split-Path -Parent $imageRoot)
    if (-not (Test-Path -LiteralPath $imageRoot)) {
        $staging = Get-ClientPath ($imageRoot + '.staging')
        if (Test-Path -LiteralPath $staging) {
            Assert-ClientTree $staging
            Assert-ClientProtected $staging
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
        New-Item -ItemType Directory -Path $staging | Out-Null
        Set-ClientProtection $staging -Private
        foreach ($entry in Get-ChildItem -LiteralPath $source) { Copy-Item -LiteralPath $entry.FullName -Destination $staging -Recurse }
        Assert-ImageSealed $staging
        Move-Item -LiteralPath $staging -Destination $imageRoot
    }
    Assert-ImageSealed $imageRoot
    $existing = Test-ClientPackage (Join-Path $imageRoot 'package')
    $incoming = Test-ClientPackage (Join-Path $source 'package')
    if (($existing | ConvertTo-Json -Depth 32 -Compress) -ne ($incoming | ConvertTo-Json -Depth 32 -Compress)) { throw 'image_package_version_conflict' }
    if ((Get-FileHash -LiteralPath (Join-Path $imageRoot 'image.json')).Hash -ne (Get-FileHash -LiteralPath (Join-Path $source 'image.json')).Hash) { throw 'image_configuration_conflict' }
    & $runtime -NoProfile -File $field -Action Upgrade -Target $targetPath
    if ($LASTEXITCODE -ne 0) { throw 'mother_upgrade_failed' }
    & $runtime -NoProfile -File $field -Action Start -Target $targetPath
} elseif ($Action -eq 'Seal') {
    & $runtime -NoProfile -File (Join-Path $source 'package/scripts/image-deployment.ps1') -Action Seal -ImageDirectory $imageRoot -ExpectedMachineName $target.machine_name -ExpectedMachineGuid $target.machine_guid
} elseif ($Action -eq 'Diagnose') {
    $output = Join-Path $bundle ('diagnostic-' + [guid]::NewGuid().ToString('N'))
    & $runtime -NoProfile -File $field -Action Diagnose -Target $targetPath -OutputDirectory $output
} else {
    & $runtime -NoProfile -File $field -Action $Action -Target $targetPath
}
if ($LASTEXITCODE -ne 0) { throw 'mother_operation_failed' }
