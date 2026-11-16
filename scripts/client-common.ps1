# Shared offline package checks and file transactions. No system changes on import.
Set-StrictMode -Version Latest

function Assert-ClientTarget {
    param([string]$ExpectedName, [string]$ExpectedGuid, [string]$ActualName, [string]$ActualGuid)
    if ($ExpectedName -notmatch '^[^\x00-\x20\\/:"*?<>|]{1,63}$' -or
        -not [guid]::TryParse($ExpectedGuid, [ref]([guid]::Empty)) -or
        $ExpectedName -ne $ActualName -or [guid]$ExpectedGuid -ne [guid]$ActualGuid) {
        throw 'target_identity_mismatch'
    }
}

function Get-ClientPath {
    param([string]$Path)
    if (-not [IO.Path]::IsPathFullyQualified($Path) -or $Path -match '["\r\n]' -or $Path.StartsWith('\\')) {
        throw 'absolute_local_path_required'
    }
    $full = [IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
    if ($full -eq [IO.Path]::GetPathRoot($full).TrimEnd('\', '/') -or $full.Substring(2).Contains(':')) {
        throw 'dedicated_directory_required'
    }
    $ancestor = $full
    while ($ancestor) {
        if ((Test-Path -LiteralPath $ancestor) -and
            ((Get-Item -Force -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'reparse_path_rejected'
        }
        $ancestor = Split-Path -Parent $ancestor
    }
    return $full
}

function Assert-ClientTree {
    param([string]$Path)
    $null = Get-ClientPath $Path
    # Enumerate each level only after checking the entry, so junctions are never traversed.
    foreach ($entry in Get-ChildItem -Force -LiteralPath $Path) {
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'reparse_tree_rejected' }
        if ($entry.PSIsContainer) { Assert-ClientTree $entry.FullName }
    }
}

function Write-ClientJson {
    param([string]$Path, $Value)
    $temporary = $Path + '.' + [guid]::NewGuid().ToString('N') + '.tmp'
    try {
        $bytes = [Text.Encoding]::UTF8.GetBytes(($Value | ConvertTo-Json -Depth 32) + "`n")
        $stream = [IO.File]::Open($temporary, 'CreateNew', 'Write', 'None')
        try { $stream.Write($bytes); $stream.Flush($true) } finally { $stream.Dispose() }
        [IO.File]::Move($temporary, $Path, $true)
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary }
    }
}

function Read-ClientJson {
    param([string]$Path)
    if ((Get-Item -LiteralPath $Path).Length -gt 8MB) { throw 'json_size_limit' }
    return Get-Content -LiteralPath $Path -Raw | ConvertFrom-Json -AsHashtable
}

function Get-ClientOrigin {
    param([string]$Value)
    $uri = [uri]$Value
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or $uri.Query -or $uri.Fragment -or $uri.AbsolutePath -ne '/') {
        throw 'https_origin_required'
    }
    return $uri.GetLeftPart([UriPartial]::Authority).TrimEnd('/')
}

function Test-ClientPackage {
    param([string]$Directory)
    $root = Get-ClientPath $Directory
    Assert-ClientTree $root
    $manifest = Read-ClientJson (Join-Path $root 'manifest.json')
    if ($manifest.schema_version -ne 1 -or $manifest.state_schema -ne 1 -or
        $manifest.version -notmatch '^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$' -or
        $manifest.version.EndsWith('.') -or
        $manifest.target -ne 'x86_64-pc-windows-msvc') { throw 'package_version_unsupported' }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        if ($file.path -notmatch '^[A-Za-z0-9_./+ -]+$' -or $file.path -match '(^|/)\.\.?(/|$)' -or
            $file.path.StartsWith('/') -or -not $seen.Add($file.path) -or
            $file.sha256 -notmatch '^[a-f0-9]{64}$') { throw 'package_manifest_invalid' }
        foreach ($part in $file.path.Split('/')) {
            if (-not $part -or $part.EndsWith('.') -or $part.EndsWith(' ') -or $part -match '^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(\.|$)') {
                throw 'package_path_invalid'
            }
        }
        $path = Join-Path $root $file.path
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or
            (Get-Item -LiteralPath $path).Length -ne $file.size -or
            (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.sha256) {
            throw ('package_hash_mismatch ' + $file.path)
        }
    }
    foreach ($required in @('bin/wccs-agent.exe', 'bin/wccs-session.exe', 'bin/wccs-inventory.exe',
        'bin/wccs-login-provider.dll', 'runtime/pwsh.exe', 'scripts/client-field.ps1',
        'scripts/client-common.ps1', 'scripts/client-system.ps1', 'scripts/waiting-service.ps1', 'scripts/login-test.ps1')) {
        if (-not $seen.Contains($required)) { throw 'package_incomplete' }
    }
    foreach ($file in Get-ChildItem -LiteralPath $root -File -Recurse -Force) {
        $relative = [IO.Path]::GetRelativePath($root, $file.FullName).Replace('\', '/')
        if ($relative -ne 'manifest.json' -and -not $seen.Contains($relative)) { throw 'package_unlisted_file' }
    }
    return $manifest
}

function Test-ClientBinaries {
    param([string]$Directory, $Manifest)
    Assert-ClientTree $Directory
    $files = @($Manifest.files | Where-Object { $_.path.StartsWith('bin/') })
    if (@(Get-ChildItem -LiteralPath $Directory -Force).Count -ne $files.Count) { throw 'installed_binary_set_mismatch' }
    foreach ($file in $files) {
        $path = Join-Path $Directory ([IO.Path]::GetFileName($file.path))
        if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or
            (Get-FileHash -LiteralPath $path).Hash -ne $file.sha256) { throw 'installed_binary_mismatch' }
    }
}

function Assert-ClientProtected {
    param([string]$Path)
    $acl = Get-Acl -LiteralPath $Path
    $trusted = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $trusted) { throw 'untrusted_owner' }
    $mask = [Security.AccessControl.FileSystemRights]::Write -bor [Security.AccessControl.FileSystemRights]::Delete -bor
        [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership -bor
        [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles
    foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
        if ($rule.AccessControlType -eq 'Allow' -and
            -not ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) -and
            ($rule.FileSystemRights -band $mask) -and $rule.IdentityReference.Value -notin $trusted) {
            throw 'unprivileged_installation_write'
        }
    }
}

function Set-ClientProtection {
    param([string]$Path, [switch]$Private)
    $acl = [Security.AccessControl.DirectorySecurity]::new()
    $acl.SetAccessRuleProtection($true, $false)
    $acl.SetOwner([Security.Principal.SecurityIdentifier]::new('S-1-5-32-544'))
    foreach ($sid in @('S-1-5-18', 'S-1-5-32-544')) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            [Security.Principal.SecurityIdentifier]::new($sid), 'FullControl', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    if (-not $Private) {
        $acl.AddAccessRule([Security.AccessControl.FileSystemAccessRule]::new(
            [Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'), 'ReadAndExecute', 'ContainerInherit,ObjectInherit', 'None', 'Allow'))
    }
    Set-Acl -LiteralPath $Path -AclObject $acl
}

function Restore-ClientBinaries {
    param([string]$Root)
    $journalPath = Join-Path $Root 'data/update.json'
    $journal = Read-ClientJson $journalPath
    if ($journal.schema_version -ne 1 -or $journal.operation -notmatch '^[a-f0-9]{32}$') { throw 'update_journal_invalid' }
    $backup = Join-Path $Root ('backup-' + $journal.operation)
    $failed = Join-Path $Root ('failed-' + $journal.operation)
    $bin = Join-Path $Root 'bin'
    # A crash between either rename is recoverable from the same journal.
    if (Test-Path -LiteralPath $backup) {
        Assert-ClientTree $backup
        Test-ClientBinaries $backup $journal.previous_manifest
        if (Test-Path -LiteralPath $bin) {
            if (Test-Path -LiteralPath $failed) { throw 'rollback_destination_conflict' }
            Move-Item -LiteralPath $bin -Destination $failed
        }
        Move-Item -LiteralPath $backup -Destination $bin
    }
    Test-ClientBinaries $bin $journal.previous_manifest
    Write-ClientJson (Join-Path $Root 'installed-package.json') $journal.previous_manifest
    Write-ClientJson (Join-Path $Root 'data/device.deployment.json') @{schema_version=1; version=$journal.previous_manifest.version}
    $journal.status = 'rolled_back'
    Write-ClientJson $journalPath $journal
}

function Update-ClientBinaries {
    param([string]$Root, [string]$Package, $Manifest)
    $current = Read-ClientJson (Join-Path $Root 'installed-package.json')
    if ($Manifest.state_schema -ne $current.state_schema) { throw 'state_version_mismatch' }
    $journalPath = Join-Path $Root 'data/update.json'
    if (Test-Path -LiteralPath $journalPath) {
        $prior = Read-ClientJson $journalPath
        if ($prior.status -notin @('committed', 'rolled_back')) { throw 'pending_update_requires_rollback' }
    }
    Test-ClientBinaries (Join-Path $Root 'bin') $current
    if ($current.version -eq $Manifest.version) {
        if (($current | ConvertTo-Json -Depth 32 -Compress) -ne ($Manifest | ConvertTo-Json -Depth 32 -Compress)) {
            throw 'immutable_version_mismatch'
        }
        return
    }
    $operation = [guid]::NewGuid().ToString('N')
    $staging = Join-Path $Root ('staging-' + $operation)
    Copy-Item -LiteralPath (Join-Path $Package 'bin') -Destination $staging -Recurse
    Test-ClientBinaries $staging $Manifest
    $journal = @{schema_version=1; operation=$operation; status='prepared'; previous_manifest=$current; next_version=$Manifest.version}
    Write-ClientJson $journalPath $journal
    try {
        Move-Item -LiteralPath (Join-Path $Root 'bin') -Destination (Join-Path $Root ('backup-' + $operation))
        Move-Item -LiteralPath $staging -Destination (Join-Path $Root 'bin')
        Test-ClientBinaries (Join-Path $Root 'bin') $Manifest
        Write-ClientJson (Join-Path $Root 'installed-package.json') $Manifest
        Write-ClientJson (Join-Path $Root 'data/device.deployment.json') @{schema_version=1; version=$Manifest.version}
        $journal.status = 'committed'
        Write-ClientJson $journalPath $journal
    } catch {
        Restore-ClientBinaries $Root
        throw 'update_failed_rolled_back'
    }
}

function Select-ClientChecks {
    param($Report)
    $steps = @('configuration','installation','server','authentication','http_heartbeat','websocket','service','session_ipc')
    $codes = @('receipt_verified','saved_offline','not_configured','invalid_configuration','connection_unavailable',
        'authentication_rejected','registration_pending','registration_rejected','incompatible_reply','service_running',
        'service_stopped','service_missing','permission_denied','session_unavailable','ipc_failed','file_missing')
    foreach ($check in $Report.checks) {
        if ($check.step -in $steps -and $check.outcome -in @('succeeded','failed','pending','unknown') -and $check.code -in $codes) {
            @{step=$check.step; outcome=$check.outcome; code=$check.code}
        }
    }
}

function Select-ClientTools {
    param($Inventory)
    $index = 0
    foreach ($tool in $Inventory.tools) {
        $index++
        $status = 'unknown'
        $reason = 'unknown'
        $check = $tool['check']
        if ($check) {
            if ($check['status'] -in @('available','unavailable')) { $status = $check['status'] }
            if ($status -eq 'available') { $reason = 'not_applicable' }
            if ($check['reason'] -in @('permission_denied','missing','not_found_in_search_scope','not_applicable','not_configured','not_a_file','unsupported','system_error')) {
                $reason = $check['reason']
            }
        }
        @{index=$index; status=$status; reason=$reason}
    }
}
