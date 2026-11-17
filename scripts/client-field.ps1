#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Verify','Collect','Preflight','Install','RollbackInstall','Upgrade','Rollback','Start','Stop','Diagnose','InitializeLogin','Recover','RemoveProvider','ExportImageLogin')][string]$Action,
    [string]$Target,
    [string]$Identity,
    [string]$CaCertificate,
    [string]$LoginConfiguration,
    [string]$ToolsConfiguration,
    [string]$OutputDirectory,
    [ValidateSet('development','lab')][string]$EvidenceContext = 'lab',
    [switch]$ForceStop,
    [switch]$RemoveCa
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'client-common.ps1')
$package = Split-Path -Parent $PSScriptRoot
$manifest = Test-ClientPackage $package
if ($Action -eq 'Verify') {
    Write-Output ("package_verified " + $manifest.version)
    return
}
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
    throw 'windows_x64_required'
}
$machineGuid = (Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Cryptography').MachineGuid
$os = Get-CimInstance Win32_OperatingSystem
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
$elevated = $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$step = 'target'
$root = $null
$lock = $null
$targetInfo = $null
$operation = [guid]::NewGuid().ToString('N')
$fieldFailed = $false

function New-EvidenceDirectory {
    if (-not $OutputDirectory) { throw 'explicit_new_output_directory_required' }
    $path = Get-ClientPath $OutputDirectory
    if (Test-Path -LiteralPath $path) { throw 'evidence_directory_already_exists' }
    New-Item -ItemType Directory -Path $path | Out-Null
    return $path
}

function Get-MatchingService {
    $service = Get-CimInstance Win32_Service -Filter "Name='$($targetInfo.service_name)'"
    $agent = Join-Path $root 'bin/wccs-agent.exe'
    $session = Join-Path $root 'bin/wccs-session.exe'
    $device = Join-Path $root 'data/device.json'
    $expected = '"' + $agent + '" --service ' + $targetInfo.service_name + ' --identity "' + $device + '" --session-exe "' + $session + '" --managed'
    if ($service -and ($service.PathName -ne $expected -or $service.StartName -ne 'LocalSystem')) {
        throw 'service_identity_mismatch'
    }
    return $service
}

function Stop-ClientService {
    $service = Get-MatchingService
    if (-not $service -or $service.State -eq 'Stopped') { return }
    # A bounded SCM request avoids Stop-Service waiting indefinitely for a hung service.
    & sc.exe stop $targetInfo.service_name *> $null
    $timer = [Diagnostics.Stopwatch]::StartNew()
    do {
        Start-Sleep -Milliseconds 250
        $service = Get-MatchingService
    } while ($service -and $service.State -ne 'Stopped' -and $timer.Elapsed.TotalSeconds -lt 30)
    if ($service -and $service.State -ne 'Stopped') {
        if (-not $ForceStop) { throw 'service_stop_timeout_use_recover_force_stop' }
        $process = Get-CimInstance Win32_Process -Filter "ProcessId=$($service.ProcessId)"
        if (-not $process -or $process.ExecutablePath -ne (Join-Path $root 'bin/wccs-agent.exe') -or
            $service.ServiceType -ne 'Own Process') { throw 'service_process_identity_mismatch' }
        Stop-Process -Id $service.ProcessId -Force
        (Get-Service -Name $targetInfo.service_name).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(15))
    }
}

function Save-SupportPackage {
    $destination = Join-Path $root ('packages/' + $manifest.version)
    if (Test-Path -LiteralPath $destination) {
        $existing = Test-ClientPackage $destination
        if (($existing | ConvertTo-Json -Depth 32 -Compress) -ne ($manifest | ConvertTo-Json -Depth 32 -Compress)) {
            throw 'immutable_support_version_mismatch'
        }
        return $destination
    }
    $staging = Join-Path $root ('package-staging-' + $operation)
    Copy-Item -LiteralPath $package -Destination $staging -Recurse
    $null = Test-ClientPackage $staging
    New-Item -ItemType Directory -Path (Join-Path $root 'packages') -Force | Out-Null
    Move-Item -LiteralPath $staging -Destination $destination
    return $destination
}

function Invoke-SystemOperation {
    param([string]$SystemAction)
    $support = Save-SupportPackage
    $systemOperation = [guid]::NewGuid().ToString('N')
    Write-ClientJson (Join-Path $root 'data/system-request.json') @{
        operation=$systemOperation; parent_operation=$operation; action=$SystemAction; machine_name=$targetInfo.machine_name; machine_guid=$targetInfo.machine_guid
    }
    $taskName = 'WCCS-Field-' + $systemOperation
    $arguments = '-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "' +
        (Join-Path $support 'scripts/client-system.ps1') + '" -InstallDirectory "' + $root + '"'
    $taskAction = New-ScheduledTaskAction -Execute (Join-Path $support 'runtime/pwsh.exe') -Argument $arguments
    $taskPrincipal = New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest
    $settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Minutes 3) -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
    Register-ScheduledTask -TaskName $taskName -Action $taskAction -Principal $taskPrincipal -Settings $settings | Out-Null
    try {
        Start-ScheduledTask -TaskName $taskName
        $timer = [Diagnostics.Stopwatch]::StartNew()
        $resultPath = Join-Path $root 'data/system-result.json'
        do {
            Start-Sleep -Milliseconds 500
            if (Test-Path -LiteralPath $resultPath) {
                $result = Read-ClientJson $resultPath
                if ($result.operation -eq $systemOperation) {
                    if (-not $result.success) { throw 'system_operation_failed_see_diagnose' }
                    return
                }
            }
        } while ($timer.Elapsed.TotalSeconds -lt 180)
        throw 'system_operation_timeout_recover_before_start'
    } finally {
        Stop-ScheduledTask -TaskName $taskName -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
        $secret = Join-Path $root 'data/recovery.secret'
        if (Test-Path -LiteralPath $secret) {
            [IO.File]::WriteAllBytes($secret, [byte[]]::new((Get-Item -LiteralPath $secret).Length))
            Remove-Item -LiteralPath $secret
        }
    }
}

function Export-ClientEvidence {
    param([switch]$Installed)
    $destination = New-EvidenceDirectory
    $evidence = [ordered]@{
        schema_version=1; operation=$operation; action=$Action; collected_at=(Get-Date).ToUniversalTime().ToString('o')
        machine_name=$env:COMPUTERNAME; machine_guid=$machineGuid; package_version=$manifest.version
        os_build=[int]$os.BuildNumber; elevated=$elevated; powershell=$PSVersionTable.PSVersion.ToString()
        checks=@(); inventory='failed'; evidence_context=$EvidenceContext; tools_configuration_supplied=[bool]$ToolsConfiguration
    }
    # Raw inventory stays local. The portable zip below contains only selected evidence.
    $private = Join-Path $destination 'private'
    New-Item -ItemType Directory -Path $private | Out-Null
    if ($elevated) { Set-ClientProtection $private -Private }
    $inventoryArguments = @('--output-dir', $private, '--context', $EvidenceContext)
    if ($ToolsConfiguration) { $inventoryArguments += @('--tools-config', $ToolsConfiguration) }
    & (Join-Path $package 'bin/wccs-inventory.exe') @inventoryArguments *> $null
    if ($LASTEXITCODE -eq 0) {
        $evidence.inventory = 'collected_locally'
        $reports = @(Get-ChildItem -LiteralPath $private -Filter report.json -Recurse -File)
        if ($reports.Count -eq 1) {
            $inventory = Read-ClientJson $reports[0].FullName
            $evidence.volumes = @($inventory.volumes.entries).Count
            $evidence.tools = @(Select-ClientTools $inventory)
        }
    }
    if ($Installed) {
        try {
            $service = Get-MatchingService
            $evidence.service = if ($service) { @{state=$service.State; local_system=($service.StartName -eq 'LocalSystem'); automatic=($service.StartMode -eq 'Auto')} } else { @{state='missing'} }
        } catch { $evidence.service = @{state='identity_mismatch'} }
        try {
            $installedManifest = Read-ClientJson (Join-Path $root 'installed-package.json')
            $evidence.installed_version = $installedManifest.version
            Test-ClientBinaries (Join-Path $root 'bin') $installedManifest
            $evidence.binary_hashes = 'matched'
        } catch { $evidence.binary_hashes = 'missing_or_mismatch' }
        $evidence.policy_recovery_pending = Test-Path -LiteralPath (Join-Path $root 'data/device.waiting-policy.json')
        $evidence.login_configured = Test-Path -LiteralPath (Join-Path $root 'data/device.login-native.json')
        $evidence.login_phase = 'not_configured'
        $journalPath = Join-Path $root 'data/device.login-native.state/journal.json'
        if (Test-Path -LiteralPath $journalPath) {
            try {
                $journal = Read-ClientJson $journalPath
                $evidence.login_phase = if ($journal.phase -in @('teaching','controlled','preparing','recovering','recovery_failed','failed','logging_in')) { $journal.phase } else { 'unknown' }
                $evidence.account_recovery_pending = $null -ne $journal.original_flags
            } catch { $evidence.login_phase = 'unreadable' }
        }
        if ($evidence.login_configured) {
            try {
                $login = Read-ClientJson (Join-Path $root 'data/device.login-native.json')
                $computer = [ADSI]('WinNT://' + $env:COMPUTERNAME)
                $teaching = $computer.Children | Where-Object { $_.SchemaClassName -eq 'User' -and $_.Name.Value -eq $login.teaching_name }
                $evidence.teaching_enabled = [bool]($teaching -and -not ([int]$teaching.UserFlags.Value -band 2))
            } catch { $evidence.teaching_enabled = 'unknown' }
        }
        $evidence.ca_present = Test-Path -LiteralPath ('Cert:/LocalMachine/Root/' + $receipt.ca_thumbprint)
        if ($evidence.ca_present) {
            $certificate = Get-Item -LiteralPath ('Cert:/LocalMachine/Root/' + $receipt.ca_thumbprint)
            $evidence.ca_not_after = $certificate.NotAfter.ToUniversalTime().ToString('o')
            $evidence.ca_time_valid = (Get-Date) -ge $certificate.NotBefore -and (Get-Date) -lt $certificate.NotAfter
        }
        $tcp = [Net.Sockets.TcpClient]::new()
        $tls = $null
        try {
            $origin = [uri]$targetInfo.server
            if ($origin.Scheme -ne 'https' -or $origin.UserInfo -or $origin.Query -or $origin.Fragment) { throw 'invalid_origin' }
            if (-not $tcp.ConnectAsync($origin.DnsSafeHost, $origin.Port).Wait(3000)) { throw 'connect_timeout' }
            $tls = [Net.Security.SslStream]::new($tcp.GetStream(), $false)
            if (-not $tls.AuthenticateAsClientAsync($origin.DnsSafeHost).Wait(5000)) { throw 'tls_timeout' }
            $leaf = [Security.Cryptography.X509Certificates.X509Certificate2]::new($tls.RemoteCertificate)
            $evidence.https_certificate = @{verified=$true; thumbprint=$leaf.Thumbprint; not_after=$leaf.NotAfter.ToUniversalTime().ToString('o')}
            $leaf.Dispose()
        } catch { $evidence.https_certificate = @{verified=$false; code='tls_or_connection_failed'} }
        finally { if ($tls) { $tls.Dispose() }; $tcp.Dispose() }
        $diagnostic = Join-Path $private 'agent-diagnostic.json'
        & (Join-Path $package 'bin/wccs-agent.exe') --diagnose (Join-Path $root 'data/device.json') $targetInfo.service_name $diagnostic *> $null
        $evidence.diagnostic_exit = $LASTEXITCODE
        if (Test-Path -LiteralPath $diagnostic) {
            $report = Read-ClientJson $diagnostic
            $evidence.checks = @(Select-ClientChecks $report.report)
            $evidence.server_acknowledged = $report.server_acknowledged -eq $true
        }
        $lastResult = Join-Path $root 'data/field-result.json'
        if (Test-Path -LiteralPath $lastResult) {
            $last = Read-ClientJson $lastResult
            if ($last.action -in @('Install','RollbackInstall','Upgrade','Rollback','Start','Stop','Diagnose','InitializeLogin','Recover','RemoveProvider','Preflight') -and
                $last.operation -match '^[a-f0-9]{32}$' -and $last.step -match '^[a-z_]{1,40}$') {
                $evidence.last_operation = @{operation=$last.operation; action=$last.action; step=$last.step; success=($last.success -eq $true)}
            }
        }
    }
    $public = Join-Path $destination 'export'
    New-Item -ItemType Directory -Path $public | Out-Null
    Write-ClientJson (Join-Path $public 'evidence.json') $evidence
    Compress-Archive -LiteralPath (Join-Path $public 'evidence.json') -DestinationPath (Join-Path $destination ('evidence-' + $env:COMPUTERNAME + '-' + $operation + '.zip'))
    if ($Action -eq 'Collect') {
        Write-ClientJson (Join-Path $destination 'target.json') @{
            machine_name=$env:COMPUTERNAME; machine_guid=$machineGuid; install_directory='C:\Program Files\WCCS'
            service_name='WCCS-Agent'; server='https://192.0.2.10'; ca_sha256='REPLACE_WITH_VERIFIED_CA_SHA256'
        }
    }
    Write-Output ('evidence_exported ' + $destination)
}

try {
    if ($Action -eq 'Collect') { $step = 'collect'; Export-ClientEvidence; return }
    if ($RemoveCa -and $Action -ne 'RollbackInstall') { throw 'remove_ca_requires_rollback_install' }
    if ($ForceStop -and $Action -notin @('Recover','RemoveProvider','RollbackInstall')) { throw 'force_stop_requires_recovery_action' }
    if (-not $Target) { throw 'explicit_target_file_required' }
    $targetInfo = Read-ClientJson $Target
    Assert-ClientTarget $targetInfo.machine_name $targetInfo.machine_guid $env:COMPUTERNAME $machineGuid
    if ($targetInfo.service_name -notmatch '^WCCS-[A-Za-z0-9-]{1,60}$') { throw 'invalid_service_name' }
    $root = Get-ClientPath $targetInfo.install_directory
    if ((Split-Path -Parent $root) -ne [Environment]::GetFolderPath('ProgramFiles')) { throw 'direct_program_files_child_required' }
    if ([int]$os.BuildNumber -lt $manifest.windows_minimum_build) { throw 'windows_build_15063_required' }
    if (-not $elevated) { throw 'elevated_administrator_required' }
    Assert-ClientProtected (Split-Path -Parent $root)
    $step = 'ownership'
    if (Test-Path -LiteralPath $root) {
        Assert-ClientTree $root
        Assert-ClientProtected $root
        $receipt = Read-ClientJson (Join-Path $root 'field-installation.json')
        Assert-ClientTarget $receipt.machine_name $receipt.machine_guid $env:COMPUTERNAME $machineGuid
        if ($receipt.service_name -ne $targetInfo.service_name) { throw 'installation_service_mismatch' }
        Assert-ClientProtected (Join-Path $root 'data')
        $lock = [IO.File]::Open((Join-Path $root 'data/field.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
        $requestPath = Join-Path $root 'data/system-request.json'
        if (Test-Path -LiteralPath $requestPath) {
            $request = Read-ClientJson $requestPath
            if ($request.operation -notmatch '^[a-f0-9]{32}$') { throw 'system_request_invalid' }
            $previousTask = Get-ScheduledTask -TaskName ('WCCS-Field-' + $request.operation) -ErrorAction SilentlyContinue
            if ($Action -notin @('Diagnose','Preflight')) {
                if ($previousTask -and $previousTask.State -in @('Running','Queued')) { throw 'previous_system_task_running' }
                if ($previousTask) { Unregister-ScheduledTask -TaskName $previousTask.TaskName -Confirm:$false }
            }
        }
    } elseif ($Action -notin @('Install','Preflight')) { throw 'installation_missing' }
    if ($Action -ne 'Diagnose') {
        $existingService = Get-MatchingService
        if ($existingService -and -not (Test-Path -LiteralPath (Join-Path $root 'field-installation.json'))) { throw 'unowned_existing_service' }
    }
    if ($Action -in @('Install','Preflight')) {
        $step = 'installation_inputs'
        $null = Get-Command Register-ScheduledTask, Import-Certificate -ErrorAction Stop
        $requiredBytes = ($manifest.files | Measure-Object -Property size -Sum).Sum + 128MB
        $drive = [IO.DriveInfo]::new([IO.Path]::GetPathRoot($root))
        if ($drive.AvailableFreeSpace -lt $requiredBytes) { throw 'insufficient_installation_space' }
        if (-not $Identity -or -not $CaCertificate) { throw 'per_machine_identity_and_ca_required' }
        $seed = Read-ClientJson $Identity
        if ($seed.device_id -notmatch '^[A-Za-z0-9_-]{1,80}$' -or $seed.enrollment_token -notmatch '^[a-fA-F0-9]{64}$' -or
            @($seed.Keys | Where-Object { $_ -notin @('server','device_id','enrollment_token','image_enrollment') }).Count -gt 0) { throw 'fresh_registration_material_required' }
        if ($seed.ContainsKey('image_enrollment') -and
            ($seed.image_enrollment -isnot [bool] -or -not $seed.image_enrollment -or -not $seed.device_id.StartsWith('image-'))) {
            throw 'image_registration_material_invalid'
        }
        $targetInfo.server = Get-ClientOrigin $targetInfo.server
        if ($seed.server -and (Get-ClientOrigin $seed.server) -ne $targetInfo.server) { throw 'https_origin_mismatch' }
        if ($targetInfo.ca_sha256 -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath $CaCertificate).Hash -ne $targetInfo.ca_sha256) { throw 'ca_hash_mismatch' }
        $ca = [Security.Cryptography.X509Certificates.X509Certificate2]::new((Resolve-Path -LiteralPath $CaCertificate).Path)
        $constraints = $ca.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.19' }
        if ($ca.HasPrivateKey -or -not $constraints -or -not $constraints.CertificateAuthority -or
            (Get-Date) -lt $ca.NotBefore -or (Get-Date) -ge $ca.NotAfter) { throw 'ca_invalid_or_expired' }
        if (Test-Path -LiteralPath (Join-Path $root 'field-installation.json')) {
            if ($receipt.device_id -ne $seed.device_id -or $receipt.ca_thumbprint -ne $ca.Thumbprint -or $receipt.server -ne $targetInfo.server) { throw 'installation_configuration_mismatch' }
            $identityPath = Join-Path $root 'data/device.json'
            if (Test-Path -LiteralPath $identityPath) {
                $savedIdentity = Read-ClientJson $identityPath
                if ($savedIdentity.device_id -ne $seed.device_id -or $savedIdentity.enrollment_token -ne $seed.enrollment_token) { throw 'installation_identity_mismatch' }
            }
        }
        if ($Action -eq 'Preflight') { Write-Output 'preflight_passed'; return }
        if (-not (Test-Path -LiteralPath $root)) {
            $step = 'protected_directories'
            New-Item -ItemType Directory -Path $root | Out-Null
            Set-ClientProtection $root
            New-Item -ItemType Directory -Path (Join-Path $root 'data') | Out-Null
            Set-ClientProtection (Join-Path $root 'data') -Private
            $lock = [IO.File]::Open((Join-Path $root 'data/field.lock'), 'CreateNew', 'ReadWrite', 'None')
            $receipt = @{schema_version=1; machine_name=$env:COMPUTERNAME; machine_guid=$machineGuid; service_name=$targetInfo.service_name
                device_id=$seed.device_id; ca_thumbprint=$ca.Thumbprint; server=$targetInfo.server; complete=$false; ca_added=$false}
            Write-ClientJson (Join-Path $root 'field-installation.json') $receipt
            $command = '"' + (Join-Path $root 'bin/wccs-agent.exe') + '" --service ' + $targetInfo.service_name + ' --identity "' + (Join-Path $root 'data/device.json') + '" --session-exe "' + (Join-Path $root 'bin/wccs-session.exe') + '" --managed'
            Write-ClientJson (Join-Path $root 'installation.json') @{name=$targetInfo.service_name; command=$command; identity_source=[IO.Path]::GetFullPath($Identity)}
        }
        if ($receipt.complete) {
            $installed = Read-ClientJson (Join-Path $root 'installed-package.json')
            if ($installed.version -ne $manifest.version) { throw 'use_upgrade_for_version_change' }
            if (($installed | ConvertTo-Json -Depth 32 -Compress) -ne ($manifest | ConvertTo-Json -Depth 32 -Compress)) { throw 'immutable_version_mismatch' }
            Test-ClientBinaries (Join-Path $root 'bin') $manifest
            if (-not (Get-MatchingService)) { throw 'installed_service_missing_repair_required' }
            Write-Output 'already_installed'; return
        }
        if ($existingService -and $existingService.State -ne 'Stopped') { throw 'incomplete_installation_stop_service_before_retry' }
        $step = 'copy_and_service'
        $null = Save-SupportPackage
        & (Join-Path $PSScriptRoot 'waiting-service.ps1') -Action Install -Name $targetInfo.service_name -ExpectedMachineName $targetInfo.machine_name -ExpectedMachineGuid $targetInfo.machine_guid -InstallDirectory $root -Identity $Identity -BinaryDirectory (Join-Path $package 'bin') | Out-Null
        foreach ($name in @('wccs-inventory.exe','wccs-login-provider.dll')) {
            Copy-Item -LiteralPath (Join-Path $package ('bin/' + $name)) -Destination (Join-Path $root ('bin/' + $name)) -Force
        }
        Write-ClientJson (Join-Path $root 'installed-package.json') $manifest
        Test-ClientBinaries (Join-Path $root 'bin') $manifest
        $step = 'machine_ca'
        if (-not (Test-Path -LiteralPath ('Cert:/LocalMachine/Root/' + $ca.Thumbprint))) {
            $receipt.ca_added = $true
            Write-ClientJson (Join-Path $root 'field-installation.json') $receipt
            Import-Certificate -FilePath (Resolve-Path -LiteralPath $CaCertificate).Path -CertStoreLocation Cert:/LocalMachine/Root | Out-Null
        }
        $step = 'server_configuration'
        $connection = Join-Path $root 'data/install-connection.json'
        Write-ClientJson $connection @{schema_version=1; server=$targetInfo.server; seat_claim=$null}
        & (Join-Path $root 'bin/wccs-agent.exe') --configure (Join-Path $root 'data/device.json') $connection 0 *> $null
        if ($LASTEXITCODE -ne 0) {
            # Saving the same configuration is idempotent even after a failed installation retry.
            $status = & (Join-Path $root 'bin/wccs-agent.exe') --setup-status (Join-Path $root 'data/device.json') | ConvertFrom-Json
            if ($LASTEXITCODE -ne 0 -or $status.configuration.server -ne $targetInfo.server) { throw 'configuration_failed' }
        }
        Write-ClientJson (Join-Path $root 'data/device.deployment.json') @{schema_version=1; version=$manifest.version}
        $receipt.complete = $true
        Write-ClientJson (Join-Path $root 'field-installation.json') $receipt
        Write-Output 'installed_service_stopped_run_start_then_diagnose'
        return
    }
    $step = $Action.ToLowerInvariant()
    switch ($Action) {
        'Diagnose' { Export-ClientEvidence -Installed }
        'Stop' { Stop-ClientService }
        'Start' {
            $installed = Read-ClientJson (Join-Path $root 'installed-package.json')
            Test-ClientBinaries (Join-Path $root 'bin') $installed
            $update = Join-Path $root 'data/update.json'
            if (Test-Path -LiteralPath $update) {
                if ((Read-ClientJson $update).status -notin @('committed','rolled_back')) { throw 'pending_update_requires_rollback' }
            }
            if (-not $receipt.complete) { throw 'installation_incomplete' }
            Start-Service -Name $targetInfo.service_name
            (Get-Service -Name $targetInfo.service_name).WaitForStatus('Running', [TimeSpan]::FromSeconds(30))
        }
        'Upgrade' {
            $current = Read-ClientJson (Join-Path $root 'installed-package.json')
            if ($current.state_schema -ne $manifest.state_schema) { throw 'state_version_mismatch' }
            if ($current.version -eq $manifest.version) {
                if (($current | ConvertTo-Json -Depth 32 -Compress) -ne ($manifest | ConvertTo-Json -Depth 32 -Compress)) { throw 'immutable_version_mismatch' }
                Test-ClientBinaries (Join-Path $root 'bin') $current
                Write-Output 'already_current'
                break
            }
            Stop-ClientService
            Invoke-SystemOperation 'Recover'
            $null = Save-SupportPackage
            Update-ClientBinaries $root $package $manifest
            Write-Output 'upgrade_complete_service_stopped_run_start_then_diagnose'
        }
        'Rollback' {
            Stop-ClientService
            Invoke-SystemOperation 'Recover'
            Restore-ClientBinaries $root
            Write-Output 'rollback_complete_service_stopped_run_start_then_diagnose'
        }
        'RollbackInstall' {
            Stop-ClientService
            Invoke-SystemOperation 'Recover'
            if (Test-Path -LiteralPath (Join-Path $root 'data/login-provider-installation.json')) { Invoke-SystemOperation 'RemoveProvider' }
            & (Join-Path $PSScriptRoot 'waiting-service.ps1') -Action Uninstall -Name $targetInfo.service_name -ExpectedMachineName $targetInfo.machine_name -ExpectedMachineGuid $targetInfo.machine_guid -InstallDirectory $root | Out-Null
            if ($RemoveCa -and $receipt.ca_added) {
                $certificatePath = 'Cert:/LocalMachine/Root/' + $receipt.ca_thumbprint
                if (Test-Path -LiteralPath $certificatePath) { Remove-Item -LiteralPath $certificatePath }
            }
            $receipt.complete = $false
            Write-ClientJson (Join-Path $root 'field-installation.json') $receipt
            Write-Output 'installation_rolled_back_protected_data_retained'
        }
        'Recover' { Stop-ClientService; Invoke-SystemOperation 'Recover'; Write-Output 'teaching_recovered_service_stopped' }
        'RemoveProvider' { Stop-ClientService; Invoke-SystemOperation 'RemoveProvider'; Write-Output 'provider_removed_service_stopped' }
        'ExportImageLogin' { Stop-ClientService; Invoke-SystemOperation 'Recover'; Invoke-SystemOperation 'ExportImageLogin'; Write-Output 'image_login_template_exported' }
        'InitializeLogin' {
            if (-not $LoginConfiguration) { throw 'login_configuration_required' }
            $configuration = Read-ClientJson $LoginConfiguration
            Assert-ClientTarget $configuration.machine_name $configuration.machine_guid $env:COMPUTERNAME $machineGuid
            $null = Get-ClientPath $configuration.work_directory
            Stop-ClientService
            Invoke-SystemOperation 'Recover'
            Write-ClientJson (Join-Path $root 'data/native-input.json') $configuration
            $password = Read-Host -Prompt 'Recovery administrator password (existing local account)' -AsSecureString
            $bytes = $null
            $pointer = [Runtime.InteropServices.Marshal]::SecureStringToGlobalAllocUnicode($password)
            try {
                $bytes = [byte[]]::new(($password.Length + 1) * 2)
                [Runtime.InteropServices.Marshal]::Copy($pointer, $bytes, 0, $password.Length * 2)
                [IO.File]::WriteAllBytes((Join-Path $root 'data/recovery.secret'), $bytes)
            } finally {
                if ($null -ne $bytes) { [Array]::Clear($bytes) }
                [Runtime.InteropServices.Marshal]::ZeroFreeGlobalAllocUnicode($pointer)
                $password.Dispose()
            }
            Invoke-SystemOperation 'InitializeLogin'
            Write-Output 'login_initialized_service_stopped_run_start'
        }
    }
} catch {
    $fieldFailed = $true
    # The console identifies the failed step. Original exceptions stay out of exported diagnostics.
    Write-Error -Message ('field_failed step=' + $step + ' code=' + $_.FullyQualifiedErrorId.Split(',')[0]) -ErrorAction Continue
    exit 1
} finally {
    if ($lock) {
        try {
            $secret = Join-Path $root 'data/recovery.secret'
            if ($Action -eq 'InitializeLogin' -and (Test-Path -LiteralPath $secret)) {
                [IO.File]::WriteAllBytes($secret, [byte[]]::new((Get-Item -LiteralPath $secret).Length))
                Remove-Item -LiteralPath $secret
            }
            Write-ClientJson (Join-Path $root 'data/field-result.json') @{
                operation=$operation; action=$Action; step=$step; success=(-not $fieldFailed); time=(Get-Date).ToUniversalTime().ToString('o')
            }
        } finally { $lock.Dispose() }
    }
}
