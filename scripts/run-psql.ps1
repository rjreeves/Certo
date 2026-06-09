# run-psql.ps1
# Open an interactive psql session (or run a single command/file).
#
# Usage:
#   .\run-psql.ps1
#   .\run-psql.ps1 -Server myserver -User postgres -Password secret
#   .\run-psql.ps1 -Command "SELECT version();"
#   .\run-psql.ps1 -File .\migration.sql

param(
    [string]$Server   = "localhost",
    [int]   $Port     = 5432,
    [string]$Database = "postgres",
    [string]$User     = "postgres",
    [string]$Password = "",
    [string]$Command  = "",
    [string]$File     = ""
)

# Locate psql
$psql = (Get-Command psql -ErrorAction SilentlyContinue)?.Source
if (-not $psql) {
    foreach ($c in @(
        "C:\Program Files\PostgreSQL\9.1\bin\psql.exe",
        "C:\Program Files (x86)\PostgreSQL\9.1\bin\psql.exe"
    )) {
        if (Test-Path $c) { $psql = $c; break }
    }
}
if (-not $psql) {
    Write-Error "psql not found. Install PostgreSQL client tools or add its bin\ to PATH."
    exit 1
}

$env:PGPASSWORD = $Password

$args = @(
    "--host=$Server",
    "--port=$Port",
    "--username=$User",
    "--dbname=$Database"
)

if ($Command) {
    $args += "--command=$Command"
} elseif ($File) {
    if (-not (Test-Path $File)) { Write-Error "File not found: $File"; exit 1 }
    $args += "--file=$File"
}

& $psql @args
exit $LASTEXITCODE
