#Requires -Version 7.0
[CmdletBinding()]
param([Parameter(Mandatory)][string]$InstallDirectory)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'client-common.ps1')
if ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18') { throw 'system_required' }
$root = Get-ClientPath $InstallDirectory
Assert-ClientProtected $root
Assert-ClientProtected (Join-Path $root 'data')
Assert-ClientTree (Join-Path $root 'data')
$systemLock = [IO.File]::Open((Join-Path $root 'data/system.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
$request = Read-ClientJson (Join-Path $root 'data/system-request.json')
$machineGuid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
Assert-ClientTarget $request.machine_name $request.machine_guid $env:COMPUTERNAME $machineGuid
$receipt = Read-ClientJson (Join-Path $root 'field-installation.json')
Assert-ClientTarget $receipt.machine_name $receipt.machine_guid $env:COMPUTERNAME $machineGuid
$package = Split-Path -Parent $PSScriptRoot
$null = Test-ClientPackage $package
$installation = Read-ClientJson (Join-Path $root 'installation.json')
$service = Get-CimInstance Win32_Service -Filter "Name='$($receipt.service_name)'"
if ($service -and ($service.PathName -ne $installation.command -or $service.State -ne 'Stopped')) {
    throw 'matching_service_must_be_stopped'
}
$result = @{schema_version=1; operation=$request.operation; action=$request.action; success=$false; code='system_operation_failed'}
try {
    switch ($request.action) {
        'ExportImageLogin' {
            $destination = Join-Path $root 'data/image-template'
            & (Join-Path $root 'bin/wccs-agent.exe') --export-login-template (Join-Path $root 'data/device.json') (Join-Path $root 'bin/wccs-session.exe') $destination *> $null
            if ($LASTEXITCODE -ne 0) { throw 'image_template_export_failed' }
            $template = Read-ClientJson (Join-Path $destination 'template.json')
            Assert-ClientUserProfile $template.journal.account_sid
        }
        'InitializeLogin' {
            & (Join-Path $PSScriptRoot 'login-test.ps1') -Action Install -ExpectedMachineName $request.machine_name -ExpectedMachineGuid $request.machine_guid -InstallDirectory $root -ProviderDll (Join-Path $root 'bin/wccs-login-provider.dll') -Configuration (Join-Path $root 'data/native-input.json') -RecoverySecretFile (Join-Path $root 'data/recovery.secret') | Out-Null
        }
        'Recover' {
            # The verified recovery copy also works when the installed executable is damaged.
            & (Join-Path $package 'bin/wccs-agent.exe') --recover-login-native (Join-Path $root 'data/device.json') (Join-Path $package 'bin/wccs-session.exe') *> $null
            if ($LASTEXITCODE -ne 0) { throw 'native_recovery_failed' }
        }
        'RemoveProvider' {
            & (Join-Path $PSScriptRoot 'login-test.ps1') -Action RemoveProvider -ExpectedMachineName $request.machine_name -ExpectedMachineGuid $request.machine_guid -InstallDirectory $root -RecoveryBinaryDirectory (Join-Path $package 'bin') | Out-Null
        }
        default { throw 'unsupported_system_action' }
    }
    $result.success = $true
    $result.code = 'completed'
} catch {
    # Exceptions may contain input values. The portable receipt uses a fixed failure code.
    $result.code = 'recovery_or_provision_failed_inspect_local_state'
    [IO.File]::WriteAllText((Join-Path $root 'data/system-error.txt'), $_.ToString())
} finally {
    $secret = Join-Path $root 'data/recovery.secret'
    if (Test-Path -LiteralPath $secret) {
        $length = (Get-Item -LiteralPath $secret).Length
        [IO.File]::WriteAllBytes($secret, [byte[]]::new($length))
        Remove-Item -LiteralPath $secret
    }
    Write-ClientJson (Join-Path $root 'data/system-result.json') $result
    $systemLock.Dispose()
}
if (-not $result.success) { exit 1 }
