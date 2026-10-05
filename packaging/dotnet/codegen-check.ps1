<#
.SYNOPSIS
  Generate C# from sample/queries.ql for SQLite and PostgreSQL, compile it, and run it against
  real databases (SQLite always; PostgreSQL when CERTO_TEST_PG_URL is set).
#>
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..\..')
$proj = Join-Path $PSScriptRoot 'Certo.Codegen.Check'
$gen = Join-Path $proj 'generated'

Push-Location $root
try { & cargo build -p certo; if ($LASTEXITCODE) { throw 'cargo build failed' } } finally { Pop-Location }
$exe = Join-Path $root ('target\debug\certo' + $(if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }))

Remove-Item -Recurse -Force $gen -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $gen | Out-Null
$schema = Join-Path $proj 'sample\schema.sdl'
$queries = Join-Path $proj 'sample\queries.ql'
$empty = Join-Path $gen 'empty.sdl'
Set-Content -Path $empty -Value ''

foreach ($d in @(@{ name = 'Sqlite'; dialect = 'sqlite' }, @{ name = 'Pg'; dialect = 'postgres' }, @{ name = 'Mysql'; dialect = 'mysql' })) {
    $q = if ($d.dialect -eq 'mysql') { Join-Path $proj 'sample\queries.mysql.ql' } else { $queries }
    & $exe ql codegen $q --schema $schema --lang csharp --dialect $d.dialect --namespace "Sample.$($d.name)" -o (Join-Path $gen "Queries.$($d.name).cs")
    if ($LASTEXITCODE) { throw "codegen failed for $($d.dialect)" }
    & $exe sdl diff $empty $schema --sql $d.dialect | Set-Content -Path (Join-Path $gen "schema.$($d.dialect -replace 'postgres','pg').sql")
    if ($LASTEXITCODE) { throw "schema SQL failed for $($d.dialect)" }
    (Get-Content (Join-Path $proj $(if ($d.dialect -eq 'mysql') { 'Scenario.mysql.cs.template' } else { 'Scenario.cs.template' })) -Raw) -replace '@NS@', "Sample.$($d.name)" -replace '@NAME@', $d.name |
        Set-Content -Path (Join-Path $gen "Scenario.$($d.name).cs")
}
Remove-Item $empty

dotnet run --project $proj -c Release
exit $LASTEXITCODE
