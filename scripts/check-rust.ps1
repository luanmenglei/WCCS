#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) {
    throw 'This check requires Windows x64, MSVC and the Windows SDK.'
}

$repositoryRoot = Split-Path -Parent $PSScriptRoot
$target = 'x86_64-pc-windows-msvc'
$msvcVersion = '14.44.35207'
$sdkVersion = '10.0.19041.0'
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if ($LASTEXITCODE -ne 0 -or -not $installation) {
    throw 'Visual Studio C++ Build Tools were not found.'
}
Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -Arch amd64 -HostArch amd64 -DevCmdArguments "-winsdk=$sdkVersion -vcvars_ver=$msvcVersion"
if ($env:VCToolsVersion.TrimEnd('\') -ne $msvcVersion -or $env:WindowsSDKVersion.TrimEnd('\') -ne $sdkVersion) {
    throw "Required toolset $msvcVersion and SDK $sdkVersion were not selected."
}
$linker = Join-Path $env:VCToolsInstallDir 'bin/Hostx64/x64/link.exe'
$compiler = Join-Path $env:VCToolsInstallDir 'bin/Hostx64/x64/cl.exe'
$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = $linker

function Invoke-Checked {
    param([string]$Program, [string[]]$Arguments)
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE."
    }
}

Push-Location $repositoryRoot
try {
    Invoke-Checked rustc @('--version', '--verbose')
    Invoke-Checked cargo @('--version')
    Write-Output "MSVC toolset $msvcVersion, Windows SDK $sdkVersion, linker $linker"

    $smokeDirectory = Join-Path $repositoryRoot ('artifacts/toolchain-smoke/' + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $smokeDirectory -Force | Out-Null
    $cSource = Join-Path $smokeDirectory 'sdk-smoke.c'
    $cObject = Join-Path $smokeDirectory 'sdk-smoke.obj'
    $cBinary = Join-Path $smokeDirectory 'sdk-smoke.exe'
    [IO.File]::WriteAllText($cSource, "#include <windows.h>`nint main(void) { return GetCurrentProcessId() == 0; }`n")
    Invoke-Checked $compiler @('/nologo', '/W4', '/WX', $cSource, "/Fo$cObject", "/Fe$cBinary", '/link', 'kernel32.lib')
    Invoke-Checked $cBinary @()

    $rustSource = Join-Path $smokeDirectory 'rust-smoke.rs'
    $rustBinary = Join-Path $smokeDirectory 'rust-smoke.exe'
    [IO.File]::WriteAllText($rustSource, 'fn main() { println!("WCCS Rust MSVC SDK smoke OK"); }' + "`n")
    Invoke-Checked rustc @($rustSource, '--crate-name', 'wccs_smoke', '--target', $target, '-C', "linker=$linker", '-o', $rustBinary)
    Invoke-Checked $rustBinary @()

    Invoke-Checked cargo @('fmt', '--all', '--', '--check')
    Invoke-Checked cargo @('build', '--locked', '-p', 'wccs-inventory', '-p', 'wccs-session', '-p', 'wccs-protocol', '-p', 'wccs-server', '-p', 'wccs-agent', '--target', $target, '--release')
    Invoke-Checked cargo @('clippy', '--locked', '-p', 'wccs-inventory', '-p', 'wccs-session', '-p', 'wccs-protocol', '-p', 'wccs-server', '-p', 'wccs-agent', '--target', $target, '--all-targets', '--', '-D', 'warnings')
    Invoke-Checked cargo @('test', '--locked', '-p', 'wccs-inventory', '-p', 'wccs-session', '-p', 'wccs-protocol', '-p', 'wccs-server', '-p', 'wccs-agent', '--target', $target)
    & (Join-Path $PSScriptRoot 'build-login-provider.ps1')
    Write-Output "Rust checks passed. Smoke artifacts: $smokeDirectory"
} finally {
    Pop-Location
}
