#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Check','Stage','Seal','Boot')][string]$Action,
    [Parameter(Mandatory)][string]$ImageDirectory,
    [string]$ExpectedMachineName,
    [string]$ExpectedMachineGuid
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'image-common.ps1')
$imageRoot = Get-ClientPath $ImageDirectory
Assert-ImageSealed $imageRoot
$configuration = Get-ImageConfiguration (Join-Path $imageRoot 'image.json')
if ($Action -eq 'Check') { Write-Output 'image_payload_sealed'; return }
$machineGuid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
$machineName = Get-ImageMachineName
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'administrator_required' }
Assert-ClientProtected (Split-Path -Parent $imageRoot)
$operationLock = [IO.File]::Open(($imageRoot + '.operation.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
try {
$taskName = 'WCCS-Image-Bootstrap'
$package = Join-Path $imageRoot 'package'
$stateRoot = Join-Path (Split-Path -Parent $imageRoot) 'WCCS-Image-State'
$cleanupPath = $imageRoot + '.cleanup.json'
$runtime = Join-Path $package 'runtime/pwsh.exe'
$arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
    (Join-Path $package 'scripts/image-deployment.ps1') + '" -Action Boot -ImageDirectory "' + $imageRoot + '"'
if ($Action -eq 'Seal') {
    Assert-ClientTarget $ExpectedMachineName $ExpectedMachineGuid $machineName $machineGuid
    $root = Get-ClientPath $configuration.install_directory
    if ((Split-Path -Parent $root) -ne [Environment]::GetFolderPath('ProgramFiles') -or
        $imageRoot.StartsWith($root + '\', [StringComparison]::OrdinalIgnoreCase) -or $imageRoot -eq $root) {
        throw 'image_and_installation_must_be_separate'
    }
    Set-ClientProtection $imageRoot -Private
    $cleanup = @{
        schema_version=1; machine_name=$machineName; machine_guid=$machineGuid
        install_directory=$root; state_directory=$stateRoot; service_name=$configuration.service_name
    }
    if ((Test-Path -LiteralPath $root) -and -not (Test-Path -LiteralPath $cleanupPath)) {
        Assert-ClientTree $root
        Assert-ClientProtected $root
        $receipt = Read-ClientJson (Join-Path $root 'field-installation.json')
        $installedIdentity = Read-ClientJson (Join-Path $root 'data/device.json')
        Assert-ClientTarget $receipt.machine_name $receipt.machine_guid $machineName $machineGuid
        if ($receipt.service_name -ne $configuration.service_name) { throw 'mother_service_mismatch' }
        $target = Join-Path $root 'data/image-target.json'
        Write-ClientJson $target @{
            machine_name=$machineName; machine_guid=$machineGuid; install_directory=$root
            service_name=$configuration.service_name; server=$configuration.server; ca_sha256=$configuration.ca_sha256
        }
        & $runtime -NoProfile -File (Join-Path $package 'scripts/client-field.ps1') -Action ExportImageLogin -Target $target
        if ($LASTEXITCODE -ne 0) { throw 'mother_prepare_and_login_before_sealing' }
        $template = Join-Path $root 'data/image-template'
        Test-ImageLoginTemplate $template
        $destination = Join-Path $imageRoot 'login-template'
        Publish-ImageLoginTemplate $template $destination ($imageRoot + '.template.tmp')
        Set-ClientProtection $destination -Private
        Test-ImageLoginTemplate $destination
        # Existing tool paths can point into an earlier public support package.
        Copy-ImageSupportPackages (Join-Path $root 'packages') (Join-Path $imageRoot 'support-packages') (Join-Path $root 'data/support-staging')
        & $runtime -NoProfile -File (Join-Path $package 'scripts/client-field.ps1') -Action RollbackInstall -Target $target
        if ($LASTEXITCODE -ne 0) { throw 'mother_recovery_or_service_removal_failed' }
        if (Get-Service -Name $configuration.service_name -ErrorAction SilentlyContinue) { throw 'mother_service_still_present' }
        # Only the verified dedicated installation tree is removed. User profiles and work files remain.
        $verified = Get-ClientPath $root
        if ($verified -ne $configuration.install_directory) { throw 'mother_cleanup_path_changed' }
        Assert-ClientTree $verified
        if (Test-Path -LiteralPath $stateRoot) {
            $statePath = Get-ClientPath $stateRoot
            Assert-ClientTree $statePath
            Assert-ClientProtected $statePath
            $seed = Read-ClientJson (Join-Path $statePath 'device.json')
            if ($seed.device_id -ne $installedIdentity.device_id -or $seed.enrollment_token -ne $installedIdentity.enrollment_token) {
                throw 'mother_bootstrap_identity_conflict'
            }
        }
        # Persist the verified cleanup intent outside both trees before deleting any file.
        Write-ClientJson $cleanupPath $cleanup
    }
    if (Test-Path -LiteralPath $cleanupPath) {
        if (Get-Service -Name $configuration.service_name -ErrorAction SilentlyContinue) { throw 'mother_service_still_present' }
        if (Test-Path 'HKLM:/SOFTWARE/Microsoft/Windows/CurrentVersion/Authentication/Credential Providers/{954886e2-4dc2-4e85-a962-2687f7658e0c}') {
            throw 'mother_provider_present'
        }
        Test-ImageLoginTemplate (Join-Path $imageRoot 'login-template')
        Remove-ImageMotherMaterial $cleanupPath $cleanup
    }
    Test-ImageLoginTemplate (Join-Path $imageRoot 'login-template')
    $Action = 'Stage'
}
if ($Action -eq 'Stage') {
    Assert-ClientTarget $ExpectedMachineName $ExpectedMachineGuid $machineName $machineGuid
    if ((Test-Path -LiteralPath $configuration.install_directory) -or (Test-Path -LiteralPath $stateRoot)) {
        throw 'mother_contains_installation_or_identity_restore_clean_baseline_before_staging'
    }
    if (Get-Service -Name $configuration.service_name -ErrorAction SilentlyContinue) { throw 'mother_service_present' }
    if (Test-Path 'HKLM:/SOFTWARE/Microsoft/Windows/CurrentVersion/Authentication/Credential Providers/{954886e2-4dc2-4e85-a962-2687f7658e0c}') {
        throw 'mother_provider_present'
    }
    Set-ClientProtection $imageRoot -Private
    Assert-ClientProtected $imageRoot
    $existing = Get-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
    if ($existing) {
        if ($existing.Actions.Execute -ne $runtime -or $existing.Actions.Arguments -ne $arguments) { throw 'bootstrap_task_conflict' }
    } else {
        $taskAction = New-ScheduledTaskAction -Execute $runtime -Argument $arguments
        $trigger = New-ScheduledTaskTrigger -AtStartup
        $system = New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest
        $settings = New-ScheduledTaskSettingsSet -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Minutes 10) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName $taskName -Action $taskAction -Trigger $trigger -Principal $system -Settings $settings | Out-Null
    }
    Write-Output 'staged_capture_image_before_next_boot'
    if (Test-Path -LiteralPath $cleanupPath) { Remove-Item -LiteralPath $cleanupPath }
    return
}
if ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18') { throw 'system_required' }
Assert-ClientProtected $imageRoot
$null = Get-ClientPath $stateRoot
if (-not (Test-Path -LiteralPath $stateRoot)) { New-Item -ItemType Directory -Path $stateRoot | Out-Null }
Set-ClientProtection $stateRoot -Private
Assert-ClientTree $stateRoot
$lock = [IO.File]::Open((Join-Path $stateRoot 'bootstrap.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
$step = 'identity'
try {
    $identity = Initialize-ImageIdentity $stateRoot $configuration.server
    $target = Join-Path $stateRoot 'target.json'
    Write-ClientJson $target @{
        machine_name=$machineName; machine_guid=$machineGuid
        install_directory=$configuration.install_directory; service_name=$configuration.service_name
        server=$configuration.server; ca_sha256=$configuration.ca_sha256
    }
    $step = 'install'
    & $runtime -NoProfile -NonInteractive -File (Join-Path $package 'scripts/client-field.ps1') -Action Install -Target $target -Identity $identity -CaCertificate (Join-Path $imageRoot 'ca.crt') *> $null
    if ($LASTEXITCODE -ne 0) { throw 'installation_failed' }
    $step = 'support_packages'
    if (Test-Path -LiteralPath (Join-Path $imageRoot 'support-packages')) {
        Copy-ImageSupportPackages (Join-Path $imageRoot 'support-packages') (Join-Path $configuration.install_directory 'packages') (Join-Path $stateRoot 'support-staging')
    }
    $template = Join-Path $imageRoot 'login-template'
    if (Test-Path -LiteralPath $template) {
        $step = 'login_template'
        $accountTemplate = Read-ClientJson (Join-Path $template 'template.json')
        Assert-ClientUserProfile $accountTemplate.journal.account_sid
        $root = $configuration.install_directory
        $native = Join-Path $root 'data/device.login-native.json'
        if (-not (Test-Path -LiteralPath $native)) {
            & $runtime -NoProfile -NonInteractive -File (Join-Path $package 'scripts/client-field.ps1') -Action Stop -Target $target *> $null
            if ($LASTEXITCODE -ne 0) { throw 'service_stop_before_import_failed' }
            & (Join-Path $root 'bin/wccs-agent.exe') --import-login-template $template (Join-Path $root 'data/device.json') *> $null
            if ($LASTEXITCODE -ne 0) { throw 'login_template_import_failed' }
        }
        Register-ClientLoginProvider $root $machineGuid
    }
    $step = 'start'
    Write-ClientJson (Join-Path $configuration.install_directory 'data/device.image-status.json') @{code='ready'}
    & $runtime -NoProfile -NonInteractive -File (Join-Path $package 'scripts/client-field.ps1') -Action Start -Target $target *> $null
    if ($LASTEXITCODE -ne 0) { throw 'service_start_failed' }
    Write-ClientJson (Join-Path $stateRoot 'status.json') @{schema_version=1; stage='registration'; code='service_started'; image_version=$configuration.image_version}
} catch {
    Write-ClientJson (Join-Path $stateRoot 'status.json') @{schema_version=1; stage=$step; code='bootstrap_failed'; image_version=$configuration.image_version}
    if ($step -in @('support_packages','login_template','start')) {
        $device = Join-Path $configuration.install_directory 'data/device.json'
        $failureCode = switch ($step) {
            'support_packages' { 'support_packages_failed' }
            'login_template' { 'login_template_failed' }
            'start' { 'service_start_failed' }
        }
        Write-ClientJson ([IO.Path]::ChangeExtension($device,'image-status.json')) @{code=$failureCode}
        & (Join-Path $configuration.install_directory 'bin/wccs-agent.exe') --identity $device --once *> $null
    }
    throw 'image_bootstrap_failed_see_status_and_field_diagnostics'
} finally { $lock.Dispose() }
} finally { $operationLock.Dispose() }
