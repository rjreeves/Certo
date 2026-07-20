# check-postgres.ps1
# Verifies that a PostgreSQL 9.1 server is running and connectable.
#
# Exit codes:
#   0 — "Postgres is running OK"
#   1 — "Postgres is NOT running"
#
# Usage:
#   .\check-postgres.ps1
#   .\check-postgres.ps1 -Server myserver -Port 5433 -Database mydb -User myuser -Password secret

param(
    [string]$Server          = "localhost",
    [int]   $Port            = 5432,
    [string]$Database        = "postgres",
    [string]$User            = "postgres",
    [string]$Password        = "",
    [string]$ExpectedVersion = "9.1"
)

$ErrorActionPreference = "SilentlyContinue"

function Fail([string]$reason) {
    Write-Host "Postgres is NOT running" -ForegroundColor Red
    if ($reason) { Write-Host "  reason : $reason" -ForegroundColor Red }
    exit 1
}

function Pass([string]$version) {
    Write-Host "Postgres $version is running OK" -ForegroundColor Green
   # Write-Host "  version : PostgreSQL $version"
    exit 0
}

# ── 1. TCP reachability ───────────────────────────────────────────────────────
try {
    $tcp     = [System.Net.Sockets.TcpClient]::new()
    $connect = $tcp.BeginConnect($Server, $Port, $null, $null)
    $ok      = $connect.AsyncWaitHandle.WaitOne(3000)
    $connected = $ok -and $tcp.Connected
    try { $tcp.EndConnect($connect) } catch {}
    $tcp.Close()
    if (-not $connected) { Fail "no TCP connection to $Server`:$Port" }
} catch {
    Fail "TCP error: $_"
}

# ── 2. Query server version via psql ─────────────────────────────────────────
$psql = (Get-Command psql -ErrorAction SilentlyContinue)?.Source
if (-not $psql) {
    foreach ($c in @(
        "C:\Program Files\PostgreSQL\9.1\bin\psql.exe",
        "C:\Program Files (x86)\PostgreSQL\9.1\bin\psql.exe"
    )) {
        if (Test-Path $c) { $psql = $c; break }
    }
}

if (-not $psql) { Fail "psql not found — install PostgreSQL 9.1 client tools" }

$env:PGPASSWORD = $Password
$out = & $psql --host=$Server --port=$Port --username=$User --dbname=$Database `
               --no-password --tuples-only --command="SELECT version();" 2>&1

if ($LASTEXITCODE -ne 0) { Fail "psql exited $LASTEXITCODE — $($out -join ' ')" }

$banner = ($out | Where-Object { $_ -match "PostgreSQL" } | Select-Object -First 1).Trim()
if (-not $banner) { Fail "no version banner returned by server" }

# Extract major.minor from "PostgreSQL 9.1.24 on ..."
if ($banner -notmatch "PostgreSQL\s+(\d+)\.(\d+)") { Fail "cannot parse version from: $banner" }
$actual = "$($Matches[1]).$($Matches[2])"

if ($actual -ne $ExpectedVersion) { Fail "expected $ExpectedVersion, server reports $actual" }

Pass $actual


