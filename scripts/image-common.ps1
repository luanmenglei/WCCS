# Image data routines. Importing this file has no system side effects.
. (Join-Path $PSScriptRoot 'client-common.ps1')

function Get-ImageMachineName {
    # Query the running OS rather than a copied deployment file or process environment.
    $name = [Environment]::MachineName
    if ($name -notmatch '^[^\x00-\x20\\/:"*?<>|]{1,63}$') { throw 'machine_name_unavailable' }
    return $name
}

function Get-ImageConfiguration {
    param([string]$Path)
    $configuration = Read-ClientJson $Path
    $keys = @('schema_version','server','ca_sha256','install_directory','service_name','image_version')
    if ($configuration.Count -ne $keys.Count -or
        @($configuration.Keys | Where-Object { $_ -notin $keys }).Count -or
        $configuration.schema_version -ne 1 -or
        $configuration.ca_sha256 -notmatch '^[a-fA-F0-9]{64}$' -or
        $configuration.service_name -notmatch '^WCCS-[A-Za-z0-9-]{1,48}$' -or
        $configuration.image_version -notmatch '^[A-Za-z0-9._-]{1,64}$') { throw 'image_configuration_invalid' }
    $configuration.server = Get-ClientOrigin $configuration.server
    $configuration.install_directory = Get-ClientPath $configuration.install_directory
    return $configuration
}

function Initialize-ImageIdentity {
    param([string]$Directory, [string]$Server, [string]$MachineName = (Get-ImageMachineName))
    if ($MachineName -notmatch '^[A-Za-z0-9-]{1,63}$') { throw 'image_machine_name_invalid' }
    $nameBytes = [Text.Encoding]::UTF8.GetBytes($MachineName.ToUpperInvariant())
    $deviceId = 'image-' + [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($nameBytes)).ToLowerInvariant().Substring(0,32)
    $root = Get-ClientPath $Directory
    Assert-ClientTree $root
    $lock = [IO.File]::Open((Join-Path $root 'identity.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    try {
        $path = Join-Path $root 'device.json'
        if (Test-Path -LiteralPath $path) {
            $identity = Read-ClientJson $path
            if ($identity.Count -ne 4 -or $identity.image_enrollment -ne $true -or
                $identity.device_id -ne $deviceId -or
                $identity.enrollment_token -notmatch '^[a-f0-9]{64}$' -or $identity.server -ne $Server) {
                throw 'image_identity_conflict'
            }
            return $path
        }
        # Device association survives disk restoration. Enrollment credentials remain random.
        $identity = @{
            server=$Server; image_enrollment=$true
            device_id=$deviceId
            enrollment_token=[Convert]::ToHexString([Security.Cryptography.RandomNumberGenerator]::GetBytes(32)).ToLowerInvariant()
        }
        Write-ClientJson $path $identity
        return $path
    } finally { $lock.Dispose() }
}

function Assert-ImageSealed {
    param([string]$Directory)
    $root = Get-ClientPath $Directory
    Assert-ClientTree $root
    $expected = @('package','image.json','ca.crt','login-template','support-packages')
    foreach ($entry in Get-ChildItem -LiteralPath $root -Force) {
        if ($entry.Name -notin $expected) { throw 'image_contains_machine_material' }
    }
    if (Test-Path -LiteralPath (Join-Path $root 'login-template')) {
        Test-ImageLoginTemplate (Join-Path $root 'login-template')
    }
    $support = Join-Path $root 'support-packages'
    if (Test-Path -LiteralPath $support) {
        foreach ($entry in Get-ChildItem -LiteralPath $support -Force) {
            if (-not $entry.PSIsContainer) { throw 'image_support_directory_required' }
            $manifest = Test-ClientPackage $entry.FullName
            if ($entry.Name -ne $manifest.version) { throw 'image_support_version_mismatch' }
        }
    }
    $configuration = Get-ImageConfiguration (Join-Path $root 'image.json')
    $null = Test-ClientPackage (Join-Path $root 'package')
    if ((Get-FileHash -LiteralPath (Join-Path $root 'ca.crt')).Hash -ne $configuration.ca_sha256) {
        throw 'image_ca_hash_mismatch'
    }
    $ca = [Security.Cryptography.X509Certificates.X509Certificate2]::new((Join-Path $root 'ca.crt'))
    try {
        $constraints = $ca.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.19' }
        if ($ca.HasPrivateKey -or -not $constraints -or -not $constraints.CertificateAuthority -or
            (Get-Date) -lt $ca.NotBefore -or (Get-Date) -ge $ca.NotAfter) { throw 'image_ca_invalid' }
    } finally { $ca.Dispose() }
}

function Copy-ImageSupportPackages {
    param([string]$Source, [string]$Destination, [Parameter(Mandatory)][string]$StagingDirectory)
    $destinationRoot = Get-ClientPath $Destination
    $stagingRoot = Get-ClientPath $StagingDirectory
    if (-not (Test-Path -LiteralPath $stagingRoot)) { New-Item -ItemType Directory -Path $stagingRoot | Out-Null }
    if (-not (Test-Path -LiteralPath $destinationRoot)) { New-Item -ItemType Directory -Path $destinationRoot | Out-Null }
    Assert-ClientTree $Source
    foreach ($entry in Get-ChildItem -LiteralPath $Source -Force) {
        if (-not $entry.PSIsContainer) { throw 'image_support_directory_required' }
        $manifest = Test-ClientPackage $entry.FullName
        if ($entry.Name -ne $manifest.version) { throw 'image_support_version_mismatch' }
        $target = Join-Path $destinationRoot $entry.Name
        if (Test-Path -LiteralPath $target) {
            $existing = Test-ClientPackage $target
            if (($existing | ConvertTo-Json -Depth 32 -Compress) -ne ($manifest | ConvertTo-Json -Depth 32 -Compress)) { throw 'image_support_content_conflict' }
            continue
        }
        $staging = Join-Path $stagingRoot $manifest.version
        $staging = Get-ClientPath $staging
        if (Test-Path -LiteralPath $staging) {
            Assert-ClientTree $staging
            Remove-Item -LiteralPath $staging -Recurse -Force
        }
        Copy-Item -LiteralPath $entry.FullName -Destination $staging -Recurse
        $null = Test-ClientPackage $staging
        Move-Item -LiteralPath $staging -Destination $target
    }
}

function Publish-ImageLoginTemplate {
    param([string]$Source, [string]$Destination, [string]$TemporaryFile)
    Test-ImageLoginTemplate $Source
    $destinationPath = Get-ClientPath $Destination
    $temporaryPath = Get-ClientPath $TemporaryFile
    if ([IO.Path]::GetPathRoot($Source) -ne [IO.Path]::GetPathRoot($destinationPath) -or
        [IO.Path]::GetPathRoot($temporaryPath) -ne [IO.Path]::GetPathRoot($destinationPath)) { throw 'mother_image_must_be_on_installation_volume' }
    if (-not (Test-Path -LiteralPath $destinationPath)) {
        Move-Item -LiteralPath $Source -Destination $destinationPath
        return
    }
    Test-ImageLoginTemplate $destinationPath
    $previous = Read-ClientJson (Join-Path $destinationPath 'template.json')
    $incoming = Read-ClientJson (Join-Path $Source 'template.json')
    if ((ConvertTo-Json $previous.config -Compress) -ne (ConvertTo-Json $incoming.config -Compress) -or
        $previous.journal.owner -ne $incoming.journal.owner -or
        $previous.journal.account_sid -ne $incoming.journal.account_sid -or
        $previous.journal.teaching_sid -ne $incoming.journal.teaching_sid) { throw 'existing_image_template_ownership_conflict' }
    # Rebinding re-encrypts DPAPI. Refresh ciphertext for the same validated accounts,
    # publishing JSON last. A process loss leaves complete, compatible individual files.
    foreach ($name in @('contest.dpapi','recovery.dpapi','template.json')) {
        $stream = [IO.File]::Open($temporaryPath, 'Create', 'Write', 'None')
        try {
            $bytes = [IO.File]::ReadAllBytes((Join-Path $Source $name))
            $stream.Write($bytes)
            $stream.Flush($true)
        } finally { $stream.Dispose() }
        [IO.File]::Move($temporaryPath, (Join-Path $destinationPath $name), $true)
    }
}

function Remove-ImageMotherMaterial {
    param([string]$Journal, [hashtable]$Expected)
    $record = Read-ClientJson (Get-ClientPath $Journal)
    if ($record.Count -ne $Expected.Count) { throw 'image_cleanup_journal_conflict' }
    foreach ($key in $Expected.Keys) {
        if (-not $record.ContainsKey($key) -or $record[$key] -ne $Expected[$key]) { throw 'image_cleanup_journal_conflict' }
    }
    foreach ($path in @($Expected.state_directory, $Expected.install_directory)) {
        $resolved = Get-ClientPath $path
        if ($resolved -ne $path) { throw 'image_cleanup_path_changed' }
        if (Test-Path -LiteralPath $resolved) {
            Assert-ClientTree $resolved
            Assert-ClientProtected $resolved
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
}

function Test-ImageLoginTemplate {
    param([string]$Directory)
    Assert-ClientTree $Directory
    $names = @('template.json','contest.dpapi','recovery.dpapi')
    $entries = @(Get-ChildItem -LiteralPath $Directory -Force)
    if ($entries.Count -ne 3 -or @($entries | Where-Object { $_.PSIsContainer -or $_.Name -notin $names }).Count) {
        throw 'image_template_file_set_invalid'
    }
    $template = Read-ClientJson (Join-Path $Directory 'template.json')
    $fieldSets = @(
        @{value=$template; keys=@('schema_version','config','journal','tool_paths')}
        @{value=$template.config; keys=@('machine_name','machine_guid','account_name','teaching_name','recovery_name','recovery_sid','work_directory')}
        @{value=$template.journal; keys=@('schema','device','machine_guid','owner','account_sid','creating_account','teaching_sid','revision','sequence','command','phase','detail','step','original_flags','session_key','desktop_prepared','prepared_tool_paths')}
    )
    foreach ($fields in $fieldSets) {
        if ($fields.value.Count -ne $fields.keys.Count -or @($fields.value.Keys | Where-Object { $_ -notin $fields.keys }).Count) {
            throw 'image_template_unknown_material'
        }
    }
    if ($template.schema_version -ne 1 -or $template.config.machine_name -ne '' -or $template.config.machine_guid -ne '' -or
        $template.journal.device -ne 'image-template' -or $template.journal.machine_guid -ne '' -or
        $template.journal.phase -ne 'teaching' -or $null -ne $template.journal.command -or
        $null -ne $template.journal.original_flags -or $null -ne $template.journal.session_key -or
        -not $template.journal.desktop_prepared -or $template.journal.creating_account -or
        $template.journal.schema -ne 1 -or $template.journal.sequence -ne 0 -or $template.journal.revision -ne 0 -or $template.journal.step -ne 0 -or
        $template.journal.detail -ne 'image_prepared' -or @($template.tool_paths).Count -eq 0 -or
        (ConvertTo-Json -InputObject $template.tool_paths -Compress) -ne (ConvertTo-Json -InputObject $template.journal.prepared_tool_paths -Compress)) { throw 'image_template_not_sanitized' }
}
