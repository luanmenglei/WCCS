Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Read-ContestConfiguration {
    param([Parameter(Mandatory)][string]$Path)
    $configuration = Get-Content -LiteralPath $Path -Raw -Encoding utf8 | ConvertFrom-Json -AsHashtable
    if ($configuration.schema_version -ne 1 -or -not $configuration.version) {
        throw '比赛配置版本无效，请联系工作人员重新部署。'
    }
    return $configuration
}

function New-ContestHttpClient {
    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $handler.UseCookies = $false
    $client = [System.Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromSeconds(20)
    return $client
}

function Get-ContestApi {
    param([hashtable]$Configuration)
    $uri = [Uri]$Configuration.judge_url
    if (-not $uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or $uri.UserInfo -or $uri.Query -or $uri.Fragment) {
        throw '比赛服务器必须使用不含凭据的 HTTPS 地址。'
    }
    if ($Configuration.contest_id -notmatch '^[A-Za-z0-9]([A-Za-z0-9._-]{0,34}[A-Za-z0-9])?$') {
        throw '比赛编号无效。'
    }
    return $uri.AbsoluteUri.TrimEnd('/') + '/api/v4/contests/' + $Configuration.contest_id
}

function Read-ContestResponse {
    param([System.Net.Http.HttpResponseMessage]$Response)
    if (-not $Response.IsSuccessStatusCode) {
        throw "服务器拒绝请求，HTTP $([int]$Response.StatusCode)。请联系工作人员核对账号、比赛和网络。"
    }
    $body = $Response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    if ($body.Length -gt 4MB) { throw '服务器返回的数据过大。' }
    return ConvertFrom-Json -InputObject $body -AsHashtable
}
