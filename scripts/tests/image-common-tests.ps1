#Requires -Version 7.0
param([Parameter(Mandatory)][string]$Root)
$ErrorActionPreference = 'Stop'
. (Join-Path (Split-Path -Parent $PSScriptRoot) 'image-common.ps1')
$originalName = $env:COMPUTERNAME
try {
    $expectedName = [Environment]::MachineName
    $env:COMPUTERNAME = 'COPIED-MOTHER-NAME'
    if ((Get-ImageMachineName) -ne $expectedName) { throw 'machine_name_from_environment' }
} finally { $env:COMPUTERNAME = $originalName }
$identities = [Collections.Generic.HashSet[string]]::new()
$credentials = [Collections.Generic.HashSet[string]]::new()
1..32 | ForEach-Object {
    $directory = Join-Path $Root "clone-$_"
    New-Item -ItemType Directory -Path $directory | Out-Null
    $path = Initialize-ImageIdentity $directory 'https://example.test' "503-$_"
    $before = [IO.File]::ReadAllBytes($path)
    $identity = Read-ClientJson $path
    if (-not $identities.Add($identity.device_id) -or -not $credentials.Add($identity.enrollment_token)) { throw 'duplicate_clone' }
    $null = Initialize-ImageIdentity $directory 'https://example.test' "503-$_"
    if ([Convert]::ToBase64String($before) -ne [Convert]::ToBase64String([IO.File]::ReadAllBytes($path))) { throw 'retry_changed_identity' }
    $rejected = $false
    try { Initialize-ImageIdentity $directory 'https://changed.test' "503-$_" } catch { $rejected = $true }
    if (-not $rejected) { throw 'configuration_conflict_accepted' }
}
$restored = Join-Path $Root 'restored'
New-Item -ItemType Directory -Path $restored | Out-Null
$restoredIdentity = Read-ClientJson (Initialize-ImageIdentity $restored 'https://example.test' '503-1')
$originalIdentity = Read-ClientJson (Join-Path $Root 'clone-1/device.json')
if ($restoredIdentity.device_id -ne $originalIdentity.device_id -or $restoredIdentity.enrollment_token -eq $originalIdentity.enrollment_token) {
    throw 'restoration_association_or_credential_failed'
}
$dirty = Join-Path $Root 'dirty-image'
New-Item -ItemType Directory -Path $dirty | Out-Null
Write-ClientJson (Join-Path $dirty 'device.json') @{enrollment_token='synthetic-secret'}
$rejected = $false
try { Assert-ImageSealed $dirty } catch {
    $rejected = $_.Exception.Message -eq 'image_contains_machine_material'
    if ($_.ToString().Contains('synthetic-secret')) { throw 'secret_leaked' }
}
if (-not $rejected) { throw 'dirty_image_accepted' }
$templateDirectory = Join-Path $Root 'template'
New-Item -ItemType Directory -Path $templateDirectory | Out-Null
$template = @{
    schema_version=1; config=@{machine_name='';machine_guid='';account_name='Contest';teaching_name='Administrator';recovery_name='Recovery';recovery_sid='S-1-5-21-1-2-3-1001';work_directory='C:/Work'}
    tool_paths=@('C:/Tools/compiler.exe')
    journal=@{schema=1;device='image-template';machine_guid='';owner='WCCS:synthetic';account_sid='S-1-5-21-1-2-3-1002';teaching_sid='S-1-5-21-1-2-3-500';revision=0;sequence=0;step=0;detail='image_prepared';phase='teaching';command=$null;original_flags=$null;session_key=$null;desktop_prepared=$true;creating_account=$false;prepared_tool_paths=@('C:/Tools/compiler.exe')}
}
Write-ClientJson (Join-Path $templateDirectory 'template.json') $template
[IO.File]::WriteAllBytes((Join-Path $templateDirectory 'contest.dpapi'),[byte[]]@(1,2))
[IO.File]::WriteAllBytes((Join-Path $templateDirectory 'recovery.dpapi'),[byte[]]@(3,4))
Test-ImageLoginTemplate $templateDirectory
$templateCopy = Join-Path $Root 'template-copy'
Copy-Item -LiteralPath $templateDirectory -Destination $templateCopy -Recurse
[IO.File]::WriteAllBytes((Join-Path $templateCopy 'contest.dpapi'),[byte[]]@(5,6))
$copyTemporary = Join-Path $Root 'template.tmp'
Publish-ImageLoginTemplate $templateCopy $templateDirectory $copyTemporary
Publish-ImageLoginTemplate $templateCopy $templateDirectory $copyTemporary
if ([IO.File]::ReadAllBytes((Join-Path $templateDirectory 'contest.dpapi'))[0] -ne 5) { throw 'reseal_ciphertext_not_refreshed' }
$conflict = Read-ClientJson (Join-Path $templateCopy 'template.json')
$conflict.journal.owner = 'WCCS:changed'
Write-ClientJson (Join-Path $templateCopy 'template.json') $conflict
$rejected = $false
try { Publish-ImageLoginTemplate $templateCopy $templateDirectory $copyTemporary } catch { $rejected = $true }
if (-not $rejected) { throw 'reseal_ownership_conflict_accepted' }
$template.journal.command = @{device_token='synthetic-secret'}
Write-ClientJson (Join-Path $templateDirectory 'template.json') $template
$rejected = $false
try { Test-ImageLoginTemplate $templateDirectory } catch { $rejected = $_.Exception.Message -eq 'image_template_not_sanitized' }
if (-not $rejected) { throw 'active_template_accepted' }
$template.journal.command = $null
$template.device_token = 'synthetic-secret'
Write-ClientJson (Join-Path $templateDirectory 'template.json') $template
$rejected = $false
try { Test-ImageLoginTemplate $templateDirectory } catch {
    $rejected = $_.Exception.Message -eq 'image_template_unknown_material'
    if ($_.ToString().Contains('synthetic-secret')) { throw 'template_secret_leaked' }
}
if (-not $rejected) { throw 'template_private_material_accepted' }
Write-Output 'image_common_checks_passed'

# Cleanup resumes from a protected intent even after the installation receipt was deleted.
# ACL enforcement is exercised by the client-common tests. This fixture only removes its temp children.
function Assert-ClientProtected { param([string]$Path) }
$cleanupRoot = Join-Path $Root 'cleanup-install'
$cleanupState = Join-Path $Root 'cleanup-state'
New-Item -ItemType Directory -Path $cleanupRoot,$cleanupState | Out-Null
[IO.File]::WriteAllText((Join-Path $cleanupRoot 'partial'), 'partial cleanup')
$cleanup = @{schema_version=1; machine_name='503-41'; machine_guid='copied-guid'; install_directory=$cleanupRoot; state_directory=$cleanupState; service_name='WCCS-Test'}
$cleanupPath = Join-Path $Root 'cleanup.json'
Write-ClientJson $cleanupPath $cleanup
$changed = $cleanup.Clone()
$changed.install_directory = $Root
$rejected = $false
try { Remove-ImageMotherMaterial $cleanupPath $changed } catch { $rejected = $true }
if (-not $rejected -or -not (Test-Path -LiteralPath $cleanupRoot)) { throw 'cleanup_conflict_not_preserved' }
Remove-ImageMotherMaterial $cleanupPath $cleanup
Remove-ImageMotherMaterial $cleanupPath $cleanup
if ((Test-Path -LiteralPath $cleanupRoot) -or (Test-Path -LiteralPath $cleanupState)) { throw 'cleanup_retry_failed' }
Write-Output 'image_cleanup_checks_passed'
