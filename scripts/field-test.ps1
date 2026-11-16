#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidateSet('Verify', 'Install', 'Diagnose', 'PrepareLogin', 'InitializeLogin', 'Start', 'Recover', 'RemoveProvider', 'RollbackInstall')]
    [string]$Action,
    [string]$BundleDirectory = $PSScriptRoot,
    [switch]$ForceStop
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$bundle = [IO.Path]::GetFullPath($BundleDirectory)
$client = Join-Path $bundle 'client'
. (Join-Path $client 'scripts/client-common.ps1')
$bundle = Get-ClientPath $bundle
$targetPath = Join-Path $bundle 'inputs/target.json'
$target = Read-ClientJson $targetPath
if ($ForceStop -and $Action -notin @('Recover', 'RemoveProvider', 'RollbackInstall')) {
    throw 'force_stop_requires_recovery_action'
}

# Refuse the wrong machine before creating files, changing ACLs or touching accounts.
if ($Action -ne 'Verify') {
    $actualGuid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
    Assert-ClientTarget $target.machine_name $target.machine_guid $env:COMPUTERNAME $actualGuid
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw '请右键点击对应 CMD 文件，选择“以管理员身份运行”。'
    }
}

$delivery = Read-ClientJson (Join-Path $bundle 'bundle-manifest.json')
foreach ($file in $delivery.files) {
    if ($file.path -notmatch '^[A-Za-z0-9_./-]+$' -or
        $file.path -match '(^|/)\.\.?(/|$)' -or $file.path.StartsWith('/')) {
        throw 'bundle_manifest_path_invalid'
    }
    $path = Get-ClientPath (Join-Path $bundle $file.path)
    if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.sha256) {
        throw ('bundle_hash_mismatch ' + $file.path)
    }
}
$manifest = Test-ClientPackage $client
if ($Action -eq 'Verify') {
    Write-Host ('文件校验通过，客户端版本 ' + $manifest.version)
    return
}

$work = Join-Path $bundle 'work'
$evidenceRoot = Join-Path $bundle 'evidence'
$stage = 'prepare'
$exitCode = 0
$operation = [guid]::NewGuid().ToString('N')
$runName = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + $operation.Substring(0, 8)
$runDirectory = Join-Path $evidenceRoot $runName
$report = [ordered]@{
    schema_version = 1
    machine_name = $target.machine_name
    machine_guid = $target.machine_guid
    action = $Action
    operation = $operation
    package_version = $manifest.version
    success = $false
    stage = $stage
    code = 'not_completed'
}

function Invoke-FieldAction {
    param([string]$Name, [string[]]$ExtraArguments = @())
    $script:stage = $Name
    Write-Host ('正在执行 ' + $Name)
    $arguments = @('-NoLogo', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        (Join-Path $client 'scripts/client-field.ps1'), '-Action', $Name, '-Target', $targetPath)
    & (Join-Path $client 'runtime/pwsh.exe') @arguments @ExtraArguments
    if ($LASTEXITCODE -ne 0) { throw ('client_step_failed_' + $Name) }
}

function Export-FieldEvidence {
    $diagnosticDirectory = Join-Path $work ('diagnose-' + $runName)
    $diagnosticAction = if (Test-Path -LiteralPath (Join-Path $target.install_directory 'field-installation.json')) {
        'Diagnose'
    } else { 'Collect' }
    Invoke-FieldAction $diagnosticAction @('-OutputDirectory', $diagnosticDirectory)
    # Copy only the export whitelist. Private raw reports and registration files stay local.
    $export = Join-Path $diagnosticDirectory 'export'
    Assert-ClientTree $export
    Copy-Item -LiteralPath $export -Destination (Join-Path $runDirectory 'diagnostic') -Recurse
}

function Initialize-FieldWorkspace {
    param([Parameter(Mandatory)][string]$Path)
    $directory = Get-ClientPath $Path
    if (Test-Path -LiteralPath $directory) {
        if (-not (Get-Item -LiteralPath $directory).PSIsContainer) { throw 'workspace_directory_required' }
    } else {
        New-Item -ItemType Directory -Path $directory | Out-Null
    }
    $acl = Get-Acl -LiteralPath $directory
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin @('S-1-5-18', 'S-1-5-32-544')) {
        throw 'workspace_owner_requires_review'
    }
    # This dedicated workspace is shared by local teaching and contest users.
    # Keep existing files and permissions while adding ordinary-user read/write access.
    $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
        [Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'), 'Modify',
        'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    Set-Acl -LiteralPath $directory -AclObject $acl
}

function Prepare-LoginConfiguration {
    $script:stage = 'PrepareLogin'
    $recoveryName = 'WccsRecovery'
    $account = Get-LocalUser -Name $recoveryName -ErrorAction SilentlyContinue
    $ownershipPath = Join-Path $work 'recovery-account.json'
    $ownership = if (Test-Path -LiteralPath $ownershipPath) { Read-ClientJson $ownershipPath } else { $null }
    if (-not $account) {
        if ($ownership) { throw 'recovery_account_missing_review_previous_sid' }
        Write-Host '将创建工作人员恢复账户 WccsRecovery。请设置一个自己记得住的密码。'
        $password = Read-Host '新账户密码（输入时不显示）' -AsSecureString
        try {
            $account = New-LocalUser -Name $recoveryName -Password $password -Description 'WCCS physical test recovery administrator'
        } finally { $password.Dispose() }
        Write-ClientJson $ownershipPath @{
            machine_name = $target.machine_name
            machine_guid = $target.machine_guid
            name = $recoveryName
            sid = $account.SID.Value
        }
        $ownership = Read-ClientJson $ownershipPath
    }
    if (-not $account.Enabled) { throw 'recovery_account_disabled' }
    if ($ownership) {
        Assert-ClientTarget $ownership.machine_name $ownership.machine_guid $target.machine_name $target.machine_guid
        if ($ownership.sid -ne $account.SID.Value) { throw 'recovery_account_sid_changed' }
    }
    $administrators = Get-LocalGroup -SID 'S-1-5-32-544'
    $members = @(Get-LocalGroupMember -Group $administrators)
    if ($account.SID.Value -notin @($members | ForEach-Object { $_.SID.Value })) {
        if (-not $ownership) { throw 'existing_recovery_account_is_not_administrator' }
        Add-LocalGroupMember -Group $administrators -Member $account
    }
    $teaching = Get-LocalUser -Name 'Administrator'
    if (-not $teaching.Enabled -or -not $teaching.SID.Value.EndsWith('-500')) {
        throw 'teaching_account_requires_review'
    }
    $configuration = @{
        machine_name = $target.machine_name
        machine_guid = $target.machine_guid
        teaching_name = $teaching.Name
        recovery_name = $account.Name
        recovery_sid = $account.SID.Value
        account_name = 'WccsContest'
        work_directory = 'C:\WCCS-Work'
    }
    $configurationPath = Join-Path $work 'login.json'
    if (Test-Path -LiteralPath $configurationPath) {
        $previous = Read-ClientJson $configurationPath
        foreach ($key in $configuration.Keys) {
            if ($previous[$key] -ne $configuration[$key]) { throw 'login_configuration_changed_review_before_overwrite' }
        }
    } else { Write-ClientJson $configurationPath $configuration }
    Initialize-FieldWorkspace $configuration.work_directory
    Write-Host '恢复账户和登录配置已准备。请用 Windows 的“切换用户”验证 WccsRecovery 能登录。'
    Write-Host '验证后回到教学账户，再运行 04-enable-login.cmd。'
}

try {
    foreach ($directory in @((Join-Path $bundle 'inputs'), $work, $evidenceRoot)) {
        $null = Get-ClientPath $directory
        if (Test-Path -LiteralPath $directory) { Assert-ClientTree $directory }
        else { New-Item -ItemType Directory -Path $directory | Out-Null }
        Set-ClientProtection $directory -Private
    }
    # A second launcher cannot race registration, recovery or account initialization.
    $lock = [IO.File]::Open((Join-Path $work 'launcher.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    New-Item -ItemType Directory -Path $runDirectory | Out-Null
    if ($Action -in @('PrepareLogin', 'InitializeLogin') -and
        -not (Test-Path -LiteralPath (Join-Path $target.install_directory 'field-installation.json'))) {
        throw 'run_01_install_first'
    }
    switch ($Action) {
        'Install' {
            $beforeDirectory = Join-Path $work ('before-' + $runName)
            Invoke-FieldAction 'Collect' @('-OutputDirectory', $beforeDirectory)
            Copy-Item -LiteralPath (Join-Path $beforeDirectory 'export') -Destination (Join-Path $runDirectory 'before') -Recurse
            $inputs = @('-Identity', (Join-Path $bundle 'inputs/device.json'), '-CaCertificate', (Join-Path $bundle 'inputs/ca.crt'))
            Invoke-FieldAction 'Preflight' $inputs
            Invoke-FieldAction 'Install' $inputs
            Invoke-FieldAction 'Start'
            Write-Host '安装和服务启动已完成。设备还需要管理端批准注册。'
        }
        'PrepareLogin' { Prepare-LoginConfiguration }
        'InitializeLogin' {
            $configurationPath = Join-Path $work 'login.json'
            if (-not (Test-Path -LiteralPath $configurationPath)) { throw 'run_03_prepare_login_first' }
            $configuration = Read-ClientJson $configurationPath
            Assert-ClientTarget $configuration.machine_name $configuration.machine_guid $target.machine_name $target.machine_guid
            if ($configuration.work_directory -ne 'C:\WCCS-Work') { throw 'workspace_configuration_requires_review' }
            Initialize-FieldWorkspace $configuration.work_directory
            Write-Host '本步骤安装 Windows 登录组件。请先完成 WccsRecovery 的原生登录验证。'
            $confirmed = Read-Host '已经实际登录验证成功，请输入 YES'
            if ($confirmed -cne 'YES') { throw 'recovery_login_verification_pending' }
            Invoke-FieldAction 'InitializeLogin' @('-LoginConfiguration', $configurationPath)
            Invoke-FieldAction 'Start'
        }
        'Diagnose' { }
        default {
            $extra = if ($ForceStop) { @('-ForceStop') } else { @() }
            Invoke-FieldAction $Action $extra
        }
    }
    $report.stage = $stage
    $report.success = $true
    $report.code = 'command_completed_review_diagnostics'
} catch {
    $exitCode = 1
    $report.stage = $stage
    $report.code = 'step_failed_see_local_console'
    Write-Host ('执行停在 ' + $stage) -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
} finally {
    if (Test-Path -LiteralPath $runDirectory) {
        try { Export-FieldEvidence }
        catch {
            $report.diagnostic_export = 'failed'
            $exitCode = 1
            Write-Host '诊断采集未完成。请拍下窗口中的报错。' -ForegroundColor Yellow
        }
        Write-ClientJson (Join-Path $runDirectory 'operation.json') $report
        $zipPath = Join-Path $evidenceRoot ('send-back-' + $runName + '.zip')
        Compress-Archive -LiteralPath $runDirectory -DestinationPath $zipPath
        Write-Host ('请回传这个结果包 ' + $zipPath) -ForegroundColor Cyan
        Write-Host '原始记录保存在 work 文件夹，回传 ZIP 已排除注册凭据与恢复密码。'
    }
    if ((Get-Variable lock -ErrorAction SilentlyContinue) -and $null -ne $lock) { $lock.Dispose() }
}
exit $exitCode
