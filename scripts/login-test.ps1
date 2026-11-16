#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Install', 'Recover', 'RemoveProvider')][string]$Action,
    [Parameter(Mandatory)][string]$ExpectedMachineGuid,
    [Parameter(Mandatory)][ValidatePattern('^[^\x00-\x20\\/:"*?<>|]{1,63}$')][string]$ExpectedMachineName,
    [Parameter(Mandatory)][string]$InstallDirectory,
    [string]$ProviderDll,
    [string]$Configuration,
    [string]$RecoverySecretFile,
    [string]$RecoveryBinaryDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$machineGuid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
if ($env:COMPUTERNAME -ne $ExpectedMachineName -or $machineGuid -ne $ExpectedMachineGuid) {
    throw 'The explicitly authorized Windows test machine does not match.'
}
if ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value -ne 'S-1-5-18') {
    throw 'Run through the target machine one-shot SYSTEM task provided by client-field.ps1.'
}
$root = [IO.Path]::GetFullPath($InstallDirectory)
$manifest = Get-Content -LiteralPath (Join-Path $root 'installation.json') -Raw | ConvertFrom-Json
$service = Get-CimInstance Win32_Service -Filter "Name='$($manifest.name)'"
if ($service -and ($service.PathName -ne $manifest.command -or $service.State -ne 'Stopped')) {
    throw 'Stop the matching WCCS test service before installation or offline recovery.'
}
$path = $root
while ($path) {
    if ((Get-Item -LiteralPath $path).Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw 'Reparse points are unsupported for login components.'
    }
    $path = Split-Path -Parent $path
}
$agent = Join-Path $root 'bin/wccs-agent.exe'
$session = Join-Path $root 'bin/wccs-session.exe'
$identity = Join-Path $root 'data/device.json'
$configPath = Join-Path $root 'data/device.login-native.json'
$providerPath = Join-Path $root 'bin/wccs-login-provider.dll'
$classId = '{954886e2-4dc2-4e85-a962-2687f7658e0c}'
$classPath = "HKLM:/SOFTWARE/Classes/CLSID/$classId/InprocServer32"
$registrationPath = "HKLM:/SOFTWARE/Microsoft/Windows/CurrentVersion/Authentication/Credential Providers/$classId"
$recordPath = Join-Path $root 'data/login-provider-installation.json'
if ($RecoveryBinaryDirectory -and $Action -in @('Recover', 'RemoveProvider')) {
    $agent = Join-Path ([IO.Path]::GetFullPath($RecoveryBinaryDirectory)) 'wccs-agent.exe'
    $session = Join-Path ([IO.Path]::GetFullPath($RecoveryBinaryDirectory)) 'wccs-session.exe'
}

function Invoke-Agent {
    param([string[]]$Arguments)
    & $agent @Arguments
    if ($LASTEXITCODE -ne 0) { throw 'Native login operation failed. Inspect credential-free account state.' }
}

if ($Action -eq 'Recover') {
    Invoke-Agent -Arguments @('--recover-login-native', $identity, $session)
    return
}
if ($Action -eq 'RemoveProvider') {
    $recoveryFailed = $false
    try { Invoke-Agent -Arguments @('--recover-login-native', $identity, $session) }
    catch { $recoveryFailed = $true }
    $record = Get-Content -LiteralPath $recordPath -Raw | ConvertFrom-Json
    if ($record.machine_guid -ne $machineGuid -or $record.provider -ne $providerPath) {
        throw 'Provider ownership record does not match.'
    }
    if ((Test-Path $classPath) -and (Get-Item $classPath).GetValue('') -ne $providerPath) {
        throw 'Provider registration changed externally.'
    }
    if (Test-Path $registrationPath) { Remove-Item -LiteralPath $registrationPath }
    if (Test-Path $classPath) { Remove-Item -LiteralPath $classPath }
    if ($recoveryFailed) { throw 'Provider removed. Account or policy recovery still requires repair and Recover.' }
    return
}
if (-not $ProviderDll -or -not $Configuration -or -not $RecoverySecretFile) {
    throw 'Install requires the provider, explicit machine configuration and local UTF-16 secret file.'
}
$config = Get-Content -LiteralPath $Configuration -Raw | ConvertFrom-Json
if ($config.machine_guid -ne $machineGuid -or $config.machine_name -ne $ExpectedMachineName) {
    throw 'Native configuration target does not match.'
}
if (Test-Path $classPath) {
    if (-not (Test-Path $recordPath) -or (Get-Item $classPath).GetValue('') -ne $providerPath) {
        throw 'Existing provider registration is not owned by this installation.'
    }
}
# The existing service installer establishes the protected parent and bin/data ACLs.
foreach ($directory in @($root, (Join-Path $root 'bin'), (Join-Path $root 'data'))) {
    $acl = Get-Acl -LiteralPath $directory
    $owner = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
    if ($owner -notin @('S-1-5-18', 'S-1-5-32-544')) { throw 'Unexpected installation owner.' }
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        $sid = $rule.IdentityReference.Value
        $writeMask = 0x000D0156
        if ($rule.AccessControlType -eq 'Allow' -and ($rule.FileSystemRights.value__ -band $writeMask) -and $sid -notin @('S-1-5-18', 'S-1-5-32-544')) {
            throw 'Unprivileged write access to login installation.'
        }
    }
}
if ([IO.Path]::GetFullPath($Configuration) -ne $configPath) {
    if (Test-Path -LiteralPath $configPath) {
        if ((Get-FileHash -LiteralPath $Configuration).Hash -ne (Get-FileHash -LiteralPath $configPath).Hash) {
            throw 'Existing native configuration differs. Recover and review account ownership before changing it.'
        }
    } else { Copy-Item -LiteralPath $Configuration -Destination $configPath }
}
Invoke-Agent -Arguments @('--provision-login-native', $configPath, $RecoverySecretFile)
if ([IO.Path]::GetFullPath($ProviderDll) -ne $providerPath) {
    Copy-Item -LiteralPath $ProviderDll -Destination $providerPath
}
@{ machine_guid = $machineGuid; provider = $providerPath; operation = [guid]::NewGuid().ToString() } |
    ConvertTo-Json | Set-Content -LiteralPath $recordPath
New-Item -Path $classPath -Force | Out-Null
Set-Item -Path $classPath -Value $providerPath
New-ItemProperty -Path $classPath -Name ThreadingModel -Value Apartment -PropertyType String -Force | Out-Null
New-Item -Path $registrationPath -Force | Out-Null
Set-Item -Path $registrationPath -Value 'WCCS contest login'
Write-Output 'Provider registered. Start the matching service explicitly.'
