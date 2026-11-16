#Requires -Version 7.0
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path -Parent $PSScriptRoot
$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -Arch amd64 -HostArch amd64 -DevCmdArguments '-winsdk=10.0.19041.0 -vcvars_ver=14.44.35207'
$output = Join-Path $repositoryRoot 'artifacts/login-provider'
New-Item -ItemType Directory -Path $output -Force | Out-Null
Push-Location $output
try {
    & cl.exe /nologo /std:c++17 /EHsc /W4 /WX /O2 /MT /LD /DUNICODE /D_UNICODE (Join-Path $repositoryRoot 'native/credential-provider/provider.cpp') /Fewccs-login-provider.dll /link /Brepro ("/DEF:" + (Join-Path $repositoryRoot 'native/credential-provider/provider.def')) advapi32.lib ole32.lib secur32.lib user32.lib uuid.lib
    if ($LASTEXITCODE -ne 0) { throw 'Credential Provider build failed.' }
} finally {
    Pop-Location
}
