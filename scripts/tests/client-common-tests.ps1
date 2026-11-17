#Requires -Version 7.0
param([Parameter(Mandatory)][string]$Root)
$ErrorActionPreference = 'Stop'
. (Join-Path (Split-Path -Parent $PSScriptRoot) 'client-common.ps1')

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) { throw $Message }
}
function Assert-Rejected {
    param([scriptblock]$Operation)
    $rejected = $false
    try { & $Operation | Out-Null } catch { $rejected = $true }
    Assert-True $rejected 'expected_rejection'
}
function New-PackageFixture {
    param([string]$Path, [string]$Version)
    $files = @('bin/wccs-agent.exe','bin/wccs-session.exe','bin/wccs-inventory.exe','bin/wccs-login-provider.dll',
        'runtime/pwsh.exe','scripts/client-field.ps1','scripts/client-common.ps1','scripts/client-system.ps1','scripts/waiting-service.ps1','scripts/login-test.ps1')
    $records = @()
    foreach ($file in $files) {
        $destination = Join-Path $Path $file
        New-Item -ItemType Directory -Path (Split-Path -Parent $destination) -Force | Out-Null
        [IO.File]::WriteAllText($destination, $Version + '-' + $file)
        $records += @{path=$file; size=(Get-Item -LiteralPath $destination).Length; sha256=(Get-FileHash -LiteralPath $destination).Hash.ToLowerInvariant()}
    }
    $manifest = @{schema_version=1; state_schema=1; version=$Version; target='x86_64-pc-windows-msvc'; files=$records}
    Write-ClientJson (Join-Path $Path 'manifest.json') $manifest
    return $manifest
}

$guid = '171f5b54-d837-47ee-a247-3c9da08fb632'
Assert-ClientTarget 'LAB_01' $guid 'lab_01' $guid
Assert-Rejected { Assert-ClientTarget 'LAB_02' $guid 'LAB_01' $guid }
Assert-Rejected { Assert-ClientTarget 'LAB_01' $guid 'LAB_01' ([guid]::NewGuid().ToString()) }
Assert-Rejected { Assert-ClientTarget 'LAB_01' 'malformed' 'LAB_01' $guid }
Assert-True ((Get-ClientOrigin 'https://EXAMPLE.test:443/') -eq 'https://example.test') 'origin_not_canonical'
foreach ($origin in @('http://example.test','https://user:secret@example.test','https://example.test/path','https://example.test/?token=secret','relative')) {
    Assert-Rejected { Get-ClientOrigin $origin }
}
foreach ($path in @('relative','C:','\servershare','C:safe:stream','C:ad"path')) {
    Assert-Rejected { Get-ClientPath $path }
}
$outside = Join-Path $Root 'outside'
New-Item -ItemType Directory -Path $outside | Out-Null
$junction = Join-Path $Root 'junction'
New-Item -ItemType Junction -Path $junction -Target $outside | Out-Null
Assert-Rejected { Get-ClientPath (Join-Path $junction 'child') }
# Remove only this known test junction, without recursion or traversing its target.
Remove-Item -LiteralPath $junction

$packageA = Join-Path $Root 'package-a'
$packageB = Join-Path $Root 'package-b'
$first = New-PackageFixture $packageA 'v1'
$second = New-PackageFixture $packageB 'v2'
$null = Test-ClientPackage $packageA
[IO.File]::AppendAllText((Join-Path $packageA 'bin/wccs-agent.exe'), 'tamper')
Assert-Rejected { Test-ClientPackage $packageA }
$first = New-PackageFixture $packageA 'v1'
[IO.File]::WriteAllText((Join-Path $packageA 'extra.txt'), 'extra')
Assert-Rejected { Test-ClientPackage $packageA }
Remove-Item -LiteralPath (Join-Path $packageA 'extra.txt')
$bad = Read-ClientJson (Join-Path $packageA 'manifest.json')
$bad.files[0].path = '../outside'
Write-ClientJson (Join-Path $packageA 'manifest.json') $bad
Assert-Rejected { Test-ClientPackage $packageA }
$first = New-PackageFixture $packageA 'v1'
$installed = Join-Path $Root 'installed'
New-Item -ItemType Directory -Path (Join-Path $installed 'data') -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $packageA 'bin') -Destination (Join-Path $installed 'bin') -Recurse
Write-ClientJson (Join-Path $installed 'installed-package.json') $first
[IO.File]::WriteAllText((Join-Path $installed 'data/device.credential'), 'SYNTHETIC_SECRET_012345')
Update-ClientBinaries $installed $packageB $second
Test-ClientBinaries (Join-Path $installed 'bin') $second
Update-ClientBinaries $installed $packageB $second
Restore-ClientBinaries $installed
Restore-ClientBinaries $installed
Test-ClientBinaries (Join-Path $installed 'bin') $first
Assert-True ((Get-Content -LiteralPath (Join-Path $installed 'data/device.credential') -Raw) -eq 'SYNTHETIC_SECRET_012345') 'identity_changed'

# Inject the failure between the two directory renames. Use the actual production transaction.
$script:failRename = $true
function Move-Item {
    param([string]$LiteralPath, [string]$Destination)
    if ($script:failRename -and (Split-Path -Leaf $LiteralPath).StartsWith('staging-')) {
        $script:failRename = $false
        throw 'injected_rename_failure'
    }
    Microsoft.PowerShell.Management\Move-Item -LiteralPath $LiteralPath -Destination $Destination
}
Assert-Rejected { Update-ClientBinaries $installed $packageB $second }
Remove-Item Function:/Move-Item
Test-ClientBinaries (Join-Path $installed 'bin') $first
Assert-True ((Read-ClientJson (Join-Path $installed 'data/update.json')).status -eq 'rolled_back') 'rollback_not_recorded'

# Simulate process loss after moving the old directory but before publishing the new one.
$operation = [guid]::NewGuid().ToString('N')
Write-ClientJson (Join-Path $installed 'data/update.json') @{schema_version=1; operation=$operation; status='prepared'; previous_manifest=$first; next_version='v2'}
Move-Item -LiteralPath (Join-Path $installed 'bin') -Destination (Join-Path $installed ('backup-' + $operation))
Restore-ClientBinaries $installed
Test-ClientBinaries (Join-Path $installed 'bin') $first

# A locked executable causes a real Windows rename rejection or a complete atomic update.
$handle = [IO.File]::Open((Join-Path $installed 'bin/wccs-agent.exe'), 'Open', 'Read', 'None')
try {
    try { Update-ClientBinaries $installed $packageB $second } catch { }
} finally { $handle.Dispose() }
$journal = Read-ClientJson (Join-Path $installed 'data/update.json')
if ($journal.status -ne 'committed') { Restore-ClientBinaries $installed }
$expected = if ($journal.status -eq 'committed') { $second } else { $first }
Test-ClientBinaries (Join-Path $installed 'bin') $expected
if ($journal.status -eq 'committed') { Restore-ClientBinaries $installed }
$incompatible = $second.Clone()
$incompatible.state_schema = 2
Assert-Rejected { Update-ClientBinaries $installed $packageB $incompatible }

$secret = 'SYNTHETIC_SECRET_012345'
$report = @{checks=@(@{step='server'; outcome='failed'; code='connection_unavailable'; error=$secret},
    @{step=$secret; outcome='failed'; code='file_missing'})}
$safe = @(Select-ClientChecks $report) | ConvertTo-Json -Depth 8
Assert-True (-not $safe.Contains($secret)) 'diagnostic_secret_leak'
Assert-True ($safe.Contains('connection_unavailable')) 'diagnostic_step_lost'
$tools = @{tools=@(@{tool=@{path=$secret; name=$secret}; check=@{status='unavailable'; reason='missing'; detail=$secret}})}
$safeTools = @(Select-ClientTools $tools) | ConvertTo-Json -Depth 8
Assert-True (-not $safeTools.Contains($secret)) 'tool_secret_leak'
$available = @(Select-ClientTools @{tools=@(@{check=@{status='available';value=@{file_version=@{status='unavailable';reason='missing'}}}})})
Assert-True ($available[0].status -eq 'available') 'available_tool_requires_no_reason_field'
Assert-True ($available[0].reason -eq 'not_applicable') 'available_tool_reason_invalid'

# Exercise the field workspace helper in a temporary directory. ACL writes are
# captured in memory so the development machine needs no elevated permissions.
$launcher = Join-Path (Split-Path -Parent $PSScriptRoot) 'field-test.ps1'
$ast = [Management.Automation.Language.Parser]::ParseFile($launcher, [ref]$null, [ref]$null)
$definition = $ast.Find({ param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Initialize-FieldWorkspace'
}, $true)
. ([scriptblock]::Create($definition.Extent.Text))
$script:workspaceAcl = [Security.AccessControl.DirectorySecurity]::new()
$script:workspaceAcl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
$script:workspaceAcl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
    [Security.Principal.SecurityIdentifier]::new('S-1-5-18'), 'FullControl',
    'ContainerInherit,ObjectInherit', 'None', 'Allow'))
$script:workspaceAclWrites = 0
function Get-Acl { param([string]$LiteralPath) return $script:workspaceAcl }
function Set-Acl {
    param([string]$LiteralPath, $AclObject)
    $script:workspaceAcl = $AclObject
    $script:workspaceAclWrites += 1
}
$workspace = Join-Path $Root 'contest-work'
Initialize-FieldWorkspace $workspace
$savedFile = Join-Path $workspace 'saved.cpp'
[IO.File]::WriteAllText($savedFile, 'int main() {}')
Initialize-FieldWorkspace $workspace
Assert-True ((Get-Content -LiteralPath $savedFile -Raw) -eq 'int main() {}') 'workspace_files_changed'
$rules = @($script:workspaceAcl.GetAccessRules($true, $false, [Security.Principal.SecurityIdentifier]))
$users = @($rules | Where-Object { $_.IdentityReference.Value -eq 'S-1-5-32-545' })
Assert-True ($users.Count -eq 1 -and ($users[0].FileSystemRights -band [Security.AccessControl.FileSystemRights]::Modify)) 'workspace_modify_missing_or_duplicated'
Assert-True (@($rules | Where-Object { $_.IdentityReference.Value -eq 'S-1-5-18' }).Count -eq 1) 'workspace_system_acl_lost'
Assert-Rejected { Initialize-FieldWorkspace $savedFile }
$workspaceLink = Join-Path $Root 'workspace-link'
New-Item -ItemType Junction -Path $workspaceLink -Target $outside | Out-Null
Assert-Rejected { Initialize-FieldWorkspace $workspaceLink }
Remove-Item -LiteralPath $workspaceLink
Assert-True ($script:workspaceAclWrites -eq 2) 'workspace_rejection_changed_acl'
. (Join-Path (Split-Path -Parent $PSScriptRoot) 'image-common.ps1')
$supportSource = Join-Path $Root 'support-source'
$null = New-PackageFixture (Join-Path $supportSource 'old-runtime') 'old-runtime'
$supportDestination = Join-Path $Root 'support-destination'
$supportStaging = Join-Path $Root 'support-staging'
New-Item -ItemType Directory -Path (Join-Path $supportStaging 'old-runtime') -Force | Out-Null
[IO.File]::WriteAllText((Join-Path $supportStaging 'old-runtime/partial'), 'interrupted copy')
Copy-ImageSupportPackages $supportSource $supportDestination $supportStaging
Copy-ImageSupportPackages $supportSource $supportDestination $supportStaging
$null = Test-ClientPackage (Join-Path $supportDestination 'old-runtime')
[IO.File]::WriteAllText((Join-Path $supportDestination 'old-runtime/runtime/pwsh.exe'), 'external change')
Assert-Rejected { Copy-ImageSupportPackages $supportSource $supportDestination $supportStaging }
Write-Output 'client_common_checks_passed'
