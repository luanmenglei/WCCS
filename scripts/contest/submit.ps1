#requires -Version 7.4
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Configuration,
    [Parameter(Mandatory, Position = 0)][string]$Problem,
    [Parameter(Mandatory, Position = 1)][string[]]$Files,
    [string]$Language,
    [switch]$Yes,
    [switch]$Resubmit
)

. (Join-Path $PSScriptRoot 'contest-common.ps1')
function Save-SubmissionReceipt([string]$Path, [hashtable]$Receipt) {
    $temporary = $Path + '.tmp'
    $bytes = [Text.Encoding]::UTF8.GetBytes(($Receipt | ConvertTo-Json -Depth 8))
    $stream = [IO.File]::Open($temporary, [IO.FileMode]::Create, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $stream.Write($bytes)
        $stream.Flush($true)
    } finally { $stream.Dispose() }
    [IO.File]::Move($temporary, $Path, $true)
}
$client = $null
$multipart = $null
$sent = $false
$receiptPath = $null
$submissionLock = $null
try {
    $config = Read-ContestConfiguration $Configuration
    $api = Get-ContestApi $config
    if (-not $Language) { $Language = [string]$config.language_id }
    $client = New-ContestHttpClient
    $response = $client.GetAsync($config.judge_url.TrimEnd('/') + '/api/v4/user').GetAwaiter().GetResult()
    try { $user = Read-ContestResponse $response } finally { $response.Dispose() }
    if (-not $user -or -not $user.team_id -or 'team' -notin $user.roles) {
        throw '本机尚未匹配选手账号，请工作人员在管理端核对 DOMjudge 账号和 IP。'
    }
    $response = $client.GetAsync($api + '/problems').GetAwaiter().GetResult()
    try { $problems = @(Read-ContestResponse $response) } finally { $response.Dispose() }
    $matches = @($problems | Where-Object { $_.id -ceq $Problem -or $_.label -ceq $Problem })
    if ($matches.Count -ne 1) { throw '题目编号或标签未能唯一匹配，请核对本场题目。' }
    $problemId = [string]$matches[0].id
    if ($Files.Count -lt 1 -or $Files.Count -gt 32) { throw '一次提交需要 1 到 32 个源文件。' }
    $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $sources = @()
    $total = 0
    foreach ($file in $Files) {
        $item = Get-Item -LiteralPath $file
        $path = $item.FullName
        $name = [IO.Path]::GetFileName($path)
        if ($name -cnotmatch '^[a-zA-Z0-9_][a-zA-Z0-9+_.-]*$') {
            throw 'DOMjudge 要求源文件名以英文字母、数字或下划线开头，后续可包含加号、句点和减号。所在文件夹可使用中文和空格。'
        }
        if (-not $names.Add($name)) { throw '提交文件有同名项。' }
        if ($item.PSIsContainer -or $item.Length -gt 1MB) { throw '请选择不超过 1 MiB 的源文件。' }
        $bytes = [IO.File]::ReadAllBytes($path)
        $total += $bytes.Length
        if ($total -gt 1MB) { throw '提交源文件合计超过 1 MiB。' }
        $sources += @{ name = $name; bytes = $bytes; sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant() }
    }
    Write-Output "账号 $($user.username) · 比赛 $($config.contest_id) · 题目 $Problem · 语言 $Language"
    foreach ($source in $sources) { Write-Output "$($source.name) $($source.sha256)" }
    $receiptDirectory = Join-Path $PWD.ProviderPath '.wccs-submissions'
    [IO.Directory]::CreateDirectory($receiptDirectory) | Out-Null
    $identity = @($config.judge_url, $config.contest_id, $user.username, $problemId, $Language) + @($sources | ForEach-Object { $_.name; $_.sha256 })
    $fingerprint = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes(($identity | ConvertTo-Json -Compress)))).ToLowerInvariant()
    $submissionLock = [IO.File]::Open((Join-Path $receiptDirectory ($fingerprint + '.lock')), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $previous = @(Get-ChildItem -LiteralPath $receiptDirectory -Filter ($fingerprint + '-*.json') |
        Sort-Object Name -Descending | Select-Object -First 1)
    if ($previous.Count -and -not $Resubmit) {
        $record = Get-Content -LiteralPath $previous[0].FullName -Raw | ConvertFrom-Json -AsHashtable
        if ($record.state -eq 'accepted') {
            Write-Output "相同文件已提交，编号 $($record.submission_id)。确认需要再次提交时使用 -Resubmit。"
            exit 0
        }
        if ($record.state -eq 'unknown') {
            [Console]::Error.WriteLine("上次提交结果待核对。先查看比赛网页，确认需要再次发送时使用 -Resubmit。回执 $($previous[0].FullName)")
            exit 4
        }
    }
    if (-not $Yes -and (Read-Host '确认提交请输入 yes') -cne 'yes') { exit 2 }
    # Persist uncertainty before POST. A lost response never causes an automatic repeat submission.
    $receiptPath = Join-Path $receiptDirectory ($fingerprint + '-' + [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds() + '-' + [Guid]::NewGuid().ToString('N') + '.json')
    $receipt = @{ schema_version = 1; state = 'unknown'; server = $config.judge_url; contest_id = $config.contest_id;
        problem_id = $problemId; language_id = $Language; started_at = [DateTimeOffset]::UtcNow.ToString('o');
        files = @($sources | ForEach-Object { @{ name = $_.name; sha256 = $_.sha256 } }) }
    Save-SubmissionReceipt $receiptPath $receipt
    $multipart = [Net.Http.MultipartFormDataContent]::new()
    $multipart.Add([Net.Http.StringContent]::new($problemId), 'problem_id')
    $multipart.Add([Net.Http.StringContent]::new($Language), 'language_id')
    foreach ($source in $sources) { $multipart.Add([Net.Http.ByteArrayContent]::new($source.bytes), 'code[]', $source.name) }
    $sent = $true
    $response = $client.PostAsync($api + '/submissions', $multipart).GetAwaiter().GetResult()
    try {
        if ([int]$response.StatusCode -ge 400 -and [int]$response.StatusCode -lt 500) {
            $receipt.state = 'rejected'
            Save-SubmissionReceipt $receiptPath $receipt
            $sent = $false
        }
        $result = Read-ContestResponse $response
    } finally { $response.Dispose() }
    if (-not $result.id) { throw '服务器响应缺少提交编号。' }
    $receipt.state = 'accepted'
    $receipt.submission_id = [string]$result.id
    Save-SubmissionReceipt $receiptPath $receipt
    Write-Output "提交成功，编号 $($result.id)。判题结果请查看比赛网页。"
    Write-Output "回执 $receiptPath"
    exit 0
} catch {
    if ($sent) {
        [Console]::Error.WriteLine("提交结果待核对。请先在比赛网页检查提交记录，避免重复提交。回执 $receiptPath")
        exit 4
    }
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 3
} finally {
    if ($multipart) { $multipart.Dispose() }
    if ($client) { $client.Dispose() }
    if ($submissionLock) { $submissionLock.Dispose() }
}
