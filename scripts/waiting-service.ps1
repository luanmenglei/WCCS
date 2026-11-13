#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Install','Start','Stop','Status','Uninstall')][string]$Action,
    [Parameter(Mandatory)][ValidatePattern('^WCCS-[A-Za-z0-9-]{1,60}$')][string]$Name,
    [Parameter(Mandatory)][string]$InstallDirectory,
    [string]$Identity,
    [string]$BinaryDirectory = 'target/x86_64-pc-windows-msvc/release'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$installRoot = [IO.Path]::GetFullPath($InstallDirectory)
if ($installRoot -eq [IO.Path]::GetPathRoot($installRoot) -or $installRoot.Contains('"')) {
    throw 'Specify a dedicated installation directory.'
}
$ancestor = $installRoot
while ($ancestor) {
    if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw 'Installation path must not traverse reparse points.'
    }
    $ancestor = Split-Path -Parent $ancestor
}
$service = Get-CimInstance Win32_Service -Filter "Name='$Name'"
$agent = Join-Path $installRoot 'bin/wccs-agent.exe'
$session = Join-Path $installRoot 'bin/wccs-session.exe'
$identityFile = Join-Path $installRoot 'data/device.json'
$expected = '"' + $agent + '" --service ' + $Name + ' --identity "' + $identityFile + '" --session-exe "' + $session + '" --managed'
if ($service -and $service.PathName -ne $expected) {
    throw 'Existing service has a different executable or configuration. No changes made.'
}
if ($Action -eq 'Status') {
    if ($service) { $service | Select-Object Name, State, StartName, ProcessId, PathName }
    else { Write-Output "$Name is not installed." }
    return
}
$principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'SCM management requires an elevated PowerShell. Foreground testing remains available.'
}
function Invoke-ServiceTool {
    param([string[]]$Arguments)
    & sc.exe @Arguments
    if ($LASTEXITCODE -ne 0) { throw "SCM operation failed with $LASTEXITCODE." }
}
switch ($Action) {
    'Install' {
        if ($service) {
            Invoke-ServiceTool -Arguments @('failure', $Name, 'reset=', '86400', 'actions=', 'restart/10000/restart/30000/none/0')
            Write-Output "$Name already has the requested installation."
            return
        }
        if (-not $Identity) { throw 'Explicit synthetic or deployment identity is required.' }
        # A writable parent could replace the protected service directory itself.
        # Require a protected existing parent rather than changing workspace ACLs.
        $parent = Split-Path -Parent $installRoot
        $acl = Get-Acl -LiteralPath $parent
        $trusted = @('S-1-5-18', 'S-1-5-32-544', 'S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464')
        $ownerSid = $acl.GetOwner([Security.Principal.SecurityIdentifier]).Value
        if ($ownerSid -notin $trusted) { throw 'Service parent owner can replace its ACL. Select a protected parent.' }
        $writeMask = [Security.AccessControl.FileSystemRights]::Write -bor [Security.AccessControl.FileSystemRights]::Delete -bor [Security.AccessControl.FileSystemRights]::ChangePermissions -bor [Security.AccessControl.FileSystemRights]::TakeOwnership -bor [Security.AccessControl.FileSystemRights]::DeleteSubdirectoriesAndFiles
        foreach ($rule in $acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])) {
            $sid = $rule.IdentityReference.Value
            if ($rule.AccessControlType -eq 'Allow' -and -not ($rule.PropagationFlags -band [Security.AccessControl.PropagationFlags]::InheritOnly) -and ($rule.FileSystemRights -band $writeMask) -and $sid -notin $trusted) {
                throw 'Service parent permits unprivileged replacement. Use a dedicated directory under protected Program Files.'
            }
        }
        $source = [IO.Path]::GetFullPath($Identity)
        $null = Get-Content -LiteralPath $source -Raw | ConvertFrom-Json
        foreach ($binary in @('wccs-agent.exe', 'wccs-session.exe')) {
            if (-not (Test-Path -LiteralPath (Join-Path $BinaryDirectory $binary) -PathType Leaf)) { throw "Build $binary before installing." }
        }
        $manifestPath = Join-Path $installRoot 'installation.json'
        if (Test-Path -LiteralPath $installRoot) {
            if (-not (Test-Path -LiteralPath $manifestPath)) { throw 'Existing directory has no matching installation record.' }
            $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
            if ($manifest.name -ne $Name -or $manifest.command -ne $expected -or $manifest.identity_source -ne $source) {
                throw 'Existing directory belongs to a different installation.'
            }
        }
        New-Item -ItemType Directory -Path (Join-Path $installRoot 'bin'), (Join-Path $installRoot 'data') -Force | Out-Null
        @{ name = $Name; command = $expected; identity_source = $source } | ConvertTo-Json | Set-Content -LiteralPath $manifestPath
        & icacls.exe $installRoot /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-32-545:(OI)(CI)RX'
        if ($LASTEXITCODE -ne 0) { throw 'Installation ACL failed.' }
        & icacls.exe (Join-Path $installRoot 'data') /inheritance:r /grant:r '*S-1-5-18:(OI)(CI)F' '*S-1-5-32-544:(OI)(CI)F'
        if ($LASTEXITCODE -ne 0) { throw 'Identity ACL failed.' }
        Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'wccs-agent.exe') -Destination $agent
        Copy-Item -LiteralPath (Join-Path $BinaryDirectory 'wccs-session.exe') -Destination $session
        Copy-Item -LiteralPath $source -Destination $identityFile
        foreach ($extension in @('credential','pending.json','completed','rotation.json','connection.json','deployment.json')) {
            $companion = [IO.Path]::ChangeExtension($source, $extension)
            if (Test-Path -LiteralPath $companion) {
                Copy-Item -LiteralPath $companion -Destination ([IO.Path]::ChangeExtension($identityFile, $extension))
            }
        }
        # Fixed relative arguments avoid embedding staff-supplied paths in batch code.
        # The operator explicitly launches this console; installation itself opens no UI.
        $setupLauncher = '@echo off' + "`r`n" + '"%~dp0bin\wccs-agent.exe" --setup "%~dp0data\device.json"' + "`r`n" + 'pause' + "`r`n"
        [IO.File]::WriteAllText((Join-Path $installRoot 'configure.cmd'), $setupLauncher, [Text.Encoding]::ASCII)
        Invoke-ServiceTool -Arguments @('create', $Name, 'binPath=', $expected, 'start=', 'auto', 'obj=', 'LocalSystem', 'DisplayName=', $Name)
        Invoke-ServiceTool -Arguments @('failure', $Name, 'reset=', '86400', 'actions=', 'restart/10000/restart/30000/none/0')
        Write-Output "Installed $Name in $installRoot. Start it explicitly after checking the target session."
    }
    'Start' { Start-Service -Name $Name }
    'Stop' { Stop-Service -Name $Name; (Get-Service -Name $Name).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30)) }
    'Uninstall' {
        if ($service) {
            if ($service.State -ne 'Stopped') { Stop-Service -Name $Name; (Get-Service -Name $Name).WaitForStatus('Stopped', [TimeSpan]::FromSeconds(30)) }
            Invoke-ServiceTool -Arguments @('delete', $Name)
        }
        Write-Output "Service removed. Protected installation files and identity remain at $installRoot."
    }
}
