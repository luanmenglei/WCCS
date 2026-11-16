#Requires -Version 7.0
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$')][string]$Version,
    [Parameter(Mandatory)][string]$PowerShellRuntime,
    [Parameter(Mandatory)][string]$Output
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repository = Split-Path -Parent $PSScriptRoot
$runtime = (Resolve-Path -LiteralPath $PowerShellRuntime).Path
$runtimeVersion = & (Join-Path $runtime 'pwsh.exe') -NoProfile -Command '$PSVersionTable.PSVersion.ToString()'
if ($LASTEXITCODE -ne 0 -or $runtimeVersion -ne '7.6.9') { throw 'PowerShell 7.6.9 x64 is required.' }
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -Arch amd64 -HostArch amd64 -DevCmdArguments '-winsdk=10.0.19041.0 -vcvars_ver=14.44.35207'
if ($env:VCToolsVersion.TrimEnd('\') -ne '14.44.35207' -or $env:WindowsSDKVersion.TrimEnd('\') -ne '10.0.19041.0') {
    throw 'Pinned Windows build toolchain was not selected.'
}
$oldFlags = $env:RUSTFLAGS
$oldTarget = $env:CARGO_TARGET_DIR
$oldLinker = $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER
Push-Location $repository
try {
    $env:RUSTFLAGS = '-C target-feature=+crt-static -C link-arg=/Brepro'
    $env:CARGO_TARGET_DIR = Join-Path $repository 'artifacts/client-build'
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:VCToolsInstallDir 'bin/Hostx64/x64/link.exe'
    & cargo build --offline --locked --release --target x86_64-pc-windows-msvc -p wccs-agent -p wccs-session -p wccs-inventory
    if ($LASTEXITCODE -ne 0) { throw 'Client build failed.' }
    & (Join-Path $PSScriptRoot 'build-login-provider.ps1')
    & python (Join-Path $PSScriptRoot 'package-client.py') --binaries artifacts/client-build/x86_64-pc-windows-msvc/release --provider artifacts/login-provider/wccs-login-provider.dll --runtime $runtime --runtime-version $runtimeVersion --version $Version --output $Output
    if ($LASTEXITCODE -ne 0) { throw 'Client packaging failed.' }
} finally {
    $env:RUSTFLAGS = $oldFlags
    $env:CARGO_TARGET_DIR = $oldTarget
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $oldLinker
    Pop-Location
}
