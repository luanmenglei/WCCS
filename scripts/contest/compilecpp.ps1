#requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Configuration,
    [Parameter(Mandatory, Position = 0)][string]$Source,
    [Alias('o')][string]$Output
)

. (Join-Path $PSScriptRoot 'contest-common.ps1')
$temporary = $null
try {
    $config = Read-ContestConfiguration $Configuration
    $sourcePath = (Get-Item -LiteralPath $Source -ErrorAction Stop).FullName
    if ([IO.Path]::GetExtension($sourcePath) -notin @('.cpp', '.cc', '.cxx', '.c++')) {
        throw '请选择 C++ 源文件。'
    }
    $compiler = [string]$config.compiler_path
    if (-not [IO.Path]::IsPathFullyQualified($compiler) -or -not (Test-Path -LiteralPath $compiler -PathType Leaf)) {
        throw '编译器路径未配置或文件缺失，请联系工作人员。'
    }
    if (-not $Output) { $Output = [IO.Path]::ChangeExtension($sourcePath, '.exe') }
    $outputPath = [IO.Path]::GetFullPath($Output, $PWD.ProviderPath)
    if ($outputPath -eq $sourcePath -or [IO.Path]::GetExtension($outputPath) -ne '.exe') {
        throw '输出路径必须使用 .exe 扩展名。'
    }
    # Reserve a unique output beside the final executable, so failure cannot publish partial code.
    $temporary = Join-Path ([IO.Path]::GetDirectoryName($outputPath)) ('.wccs-' + [Guid]::NewGuid().ToString('N') + '.exe')
    $start = [Diagnostics.ProcessStartInfo]::new($compiler)
    $start.UseShellExecute = $false
    foreach ($argument in $config.compiler_arguments) { $start.ArgumentList.Add([string]$argument) }
    $start.ArgumentList.Add($sourcePath)
    $start.ArgumentList.Add('-o')
    $start.ArgumentList.Add($temporary)
    $process = [Diagnostics.Process]::Start($start)
    try { $process.WaitForExit(); $result = $process.ExitCode } finally { $process.Dispose() }
    if ($result -ne 0) {
        if (Test-Path -LiteralPath $outputPath) { Remove-Item -LiteralPath $outputPath -Force }
        [Console]::Error.WriteLine('编译失败，旧输出已清理。')
        exit 1
    }
    [IO.File]::Move($temporary, $outputPath, $true)
    Write-Output "编译完成 $outputPath"
    exit 0
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 2
} finally {
    if ($temporary -and (Test-Path -LiteralPath $temporary)) { Remove-Item -LiteralPath $temporary -Force }
}
