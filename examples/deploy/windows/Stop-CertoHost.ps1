param(
    [Parameter(Mandatory = $true)]
    [string]$AdminToken,
    [string]$Uri = "http://127.0.0.1:8080/admin/drain"
)

$headers = @{ Authorization = "Bearer $AdminToken" }
$response = Invoke-WebRequest -Method Post -Uri $Uri -Headers $headers -UseBasicParsing
if ($response.StatusCode -ne 202) {
    throw "Certo host rejected drain request with HTTP $($response.StatusCode)"
}
