<#
.SYNOPSIS
  Build the reference CLI (certo-db) and drive it end to end on SQLite: schemas, queries, code
  generation, and the whole migration lifecycle, checking output and exit codes.
#>
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot

# the CLI copies the native library for this machine from Certo.Native/native; stage it if it is not there
$rid = & dotnet --info | Select-String 'RID:\s*(\S+)' | ForEach-Object { $_.Matches[0].Groups[1].Value } | Select-Object -First 1
if (-not (Test-Path (Join-Path $here "Certo.Native/native/$rid/native"))) {
    & pwsh -NoProfile -File (Join-Path $here 'stage-native.ps1')
    if ($LASTEXITCODE) { throw 'staging the native library failed' }
}
& dotnet build (Join-Path $here 'Certo.Db.Cli') -c Release --nologo -v quiet
if ($LASTEXITCODE) { throw 'building certo-db failed' }
$exe = Join-Path $here ('Certo.Db.Cli/bin/Release/net8.0/certo-db' + $(if ($IsWindows -or $env:OS -eq 'Windows_NT') { '.exe' } else { '' }))

$work = Join-Path ([IO.Path]::GetTempPath()) ("certo-db-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $work | Out-Null
$failures = 0
$errFile = Join-Path $work 'stderr.txt'

function Run {
    param([string[]]$CliArgs)
    $out = & $exe @CliArgs 2>$errFile
    $script:code = $LASTEXITCODE
    $script:out = ($out | Out-String)
    $script:err = if (Test-Path $errFile) { Get-Content $errFile -Raw } else { '' }
    if ($null -eq $script:err) { $script:err = '' }
}
function Check([bool]$ok, [string]$what) {
    Write-Host ("{0} {1}" -f $(if ($ok) { 'ok  ' } else { 'FAIL' }), $what)
    if (-not $ok) { $script:failures++; Write-Host "     exit=$code`n     stdout: $out`n     stderr: $err" }
}

try {
    $schema = @'
enum Status { new, paid }
table customers { id: serial primary key  email: varchar(100) not null unique  name: text }
table orders {
    id: serial primary key
    customer_id: int not null references customers
    status: Status not null default new
    total: decimal(10,2) not null
}
index orders_status on orders (status)
'@
    $queries = @'
query by_email(e: varchar(100)) { from customers c where c.email == :e select c.id, c.name }
query spend(min: decimal(10,2)) {
    with t as (from orders o group by o.customer_id select o.customer_id, sum(o.total) as total)
    from customers c join t on t.customer_id == c.id where t.total >= :min select c.email, t.total order by t.total desc
}
insert add(e: varchar(100), n: text null) { into customers set email = :e, name = :n returning id }
'@
    Set-Content (Join-Path $work 'schema.sdl') $schema
    Set-Content (Join-Path $work 'queries.ql') $queries
    Push-Location $work

    Run @('--version')
    Check ($code -eq 0 -and $out -match 'Certo.Native 0\.') "--version names the native library: $($out.Trim())"
    Run @('--help')
    Check ($code -eq 0 -and $out -match 'ql codegen' -and $out -match 'migrate init') '--help lists the commands'

    # ---- schemas
    Run @('schema', 'check', 'schema.sdl')
    Check ($code -eq 0 -and $out -match 'table customers' -and $out -match 'id serial primary key' -and $out -match 'email varchar\(100\) not null unique' -and $out -match 'enum Status') 'schema check lists tables, columns and enums'
    Check ($out -match 'status Status not null' -and $out -match 'references customers\(id\)' -and $out -match 'index orders_status') '...with defaults, references and indexes'
    Set-Content (Join-Path $work 'bad.sdl') "table t { id: nope }"
    Run @('schema', 'check', 'bad.sdl')
    Check ($code -eq 1 -and $err -match 'bad\.sdl:1:\d+: error SDL') 'a schema error is reported as file:line:col and exits 1'
    Run @('schema', 'ir', 'schema.sdl', '-o', 'IR.json')
    Check ($code -eq 0 -and (Test-Path 'IR.json') -and (Get-Content 'IR.json' -Raw) -match '"tables"') 'schema ir writes the IR'

    Set-Content (Join-Path $work 'v2.sdl') ($schema + "`ntable notes { id: serial primary key  body: text }")
    Run @('schema', 'diff', 'schema.sdl', 'v2.sdl')
    Check ($code -eq 0 -and $out -match 'table notes') 'schema diff summarises the change'
    Run @('schema', 'diff', 'schema.sdl', 'v2.sdl', '--sql', '--dialect', 'sqlite')
    Check ($code -eq 0 -and $out -match 'CREATE TABLE "notes"' -and $out -match 'AUTOINCREMENT') 'schema diff --sql lowers for sqlite'
    Run @('schema', 'diff', 'schema.sdl', 'schema.sdl')
    Check ($code -eq 0 -and $err -match 'no changes') 'identical schemas: no changes'

    # ---- queries
    Run @('ql', 'check', 'queries.ql', '--schema', 'schema.sdl')
    Check ($code -eq 0 -and $out -match 'query by_email\(e: varchar\(100\)\)' -and $out -match '-> id int, name text\?') 'ql check shows typed parameters and result columns'
    Check ($out -match 'insert add\(e: varchar\(100\), n: text\?\)' -and $out -match '-> id int') '...mutations too'
    Run @('ql', 'compile', 'queries.ql', '--schema', 'schema.sdl', '--dialect', 'sqlite')
    Check ($code -eq 0 -and $out -match 'CAST\(\?1 AS' -and $out -match '\?1=e') 'ql compile prints sqlite SQL with ?N placeholders'
    Run @('ql', 'codegen', 'queries.ql', '--schema', 'schema.sdl', '--dialect', 'sqlite', '--namespace', 'App.Db', '-o', 'Queries.cs')
    $cs = if (Test-Path 'Queries.cs') { Get-Content 'Queries.cs' -Raw } else { '' }
    Check ($code -eq 0 -and $cs -match 'namespace App\.Db;' -and $cs -match 'ByEmailAsync' -and $cs -match 'AddAsync') 'ql codegen writes the C# file'
    Set-Content (Join-Path $work 'badq.ql') "query q() { from customers c select c.ghost }"
    Run @('ql', 'check', 'badq.ql', '--schema', 'schema.sdl')
    Check ($code -eq 1 -and $err -match 'badq\.ql:1:\d+: error QL206') 'a query error is positioned and exits 1'

    # ---- migrations
    $proj = Join-Path $work 'proj'
    New-Item -ItemType Directory $proj | Out-Null
    $db = Join-Path $proj 'app.db'
    Run @('migrate', 'init', '--dir', $proj, '--dialect', 'sqlite')
    Check ($code -eq 0 -and $out -match 'created sqlite project') 'migrate init'
    Run @('migrate', 'new', 'nothing', '--dir', $proj)
    Check ($code -eq 1 -and $err -match 'no_changes|no changes|nothing') 'a migration with no changes is refused'
    Copy-Item 'schema.sdl' (Join-Path $proj 'schema.sdl') -Force
    Run @('migrate', 'new', 'init', '--dir', $proj)
    Check ($code -eq 0 -and $out -match 'created 0001_init' -and $out -match 'table customers') 'migrate new freezes the schema'
    Run @('migrate', 'list', '--dir', $proj)
    Check ($code -eq 0 -and $out -match '0001_init') 'migrate list'
    Run @('migrate', 'status', '--dir', $proj)
    Check ($code -eq 2 -and $err -match 'no database') 'a missing database is a usage error (exit 2)'
    Run @('migrate', 'status', '--dir', $proj, '--url', $db)
    Check ($code -eq 0 -and $out -match 'pending  0001_init') 'migrate status: one pending'
    Run @('migrate', 'apply', '--dir', $proj, '--url', $db, '--dry-run')
    Check ($code -eq 0 -and $out -match '-- 0001_init' -and $out -match 'CREATE TABLE') 'migrate apply --dry-run prints the script'
    $env:DATABASE_URL = $db
    Run @('migrate', 'apply', '--dir', $proj)
    Remove-Item Env:DATABASE_URL
    Check ($code -eq 0 -and $out -match 'applied 0001_init') 'migrate apply (database from DATABASE_URL)'
    Run @('migrate', 'status', '--dir', $proj, '--url', $db)
    Check ($code -eq 0 -and $out -match 'applied  0001_init' -and $err -match 'up to date') 'migrate status: up to date'
    Run @('migrate', 'drift', '--dir', $proj, '--url', $db)
    Check ($code -eq 0 -and $out -match 'no drift') 'migrate drift: no drift'

    # a destructive change needs permission; an MDL-free rebuild then applies cleanly
    Set-Content (Join-Path $proj 'schema.sdl') ($schema -replace 'name: text', '')
    Run @('migrate', 'new', 'drop_name', '--dir', $proj)
    Check ($code -eq 1 -and $err -match 'allow-destructive') 'a destructive change is refused and says how to proceed'
    Run @('migrate', 'new', 'drop_name', '--dir', $proj, '--allow-destructive')
    Check ($code -eq 0 -and $err -match 'destructive') 'with --allow-destructive it is frozen, with a warning'
    Run @('migrate', 'apply', '--dir', $proj, '--url', $db, '--check-drift')
    Check ($code -eq 0 -and $out -match 'applied 0002_drop_name') 'migrate apply --check-drift'
    Run @('migrate', 'drift', '--dir', $proj, '--url', $db)
    Check ($code -eq 0 -and $out -match 'no drift') '...still in sync after a SQLite table rebuild'

    # a schema with errors is a compile failure with positions
    Set-Content (Join-Path $proj 'schema.sdl') 'table users { id: nope }'
    Run @('migrate', 'new', 'broken', '--dir', $proj)
    Check ($code -eq 1 -and $err -match 'schema\.sdl:1:\d+: error SDL') 'migrate new reports schema errors with positions'

    # adopt refuses a database that is already managed
    $fresh = Join-Path $work 'fresh'
    New-Item -ItemType Directory $fresh | Out-Null
    Run @('migrate', 'init', '--dir', $fresh, '--dialect', 'sqlite')
    Run @('migrate', 'adopt', '--dir', $fresh, '--url', $db)
    Check ($code -eq 1 -and $err -match 'already has a migration history|not managed yet') 'adopt refuses a managed database, with the reason'

    # ---- usage errors
    Run @('frobnicate')
    Check ($code -eq 2 -and $err -match 'unknown command') 'an unknown command exits 2'
    Run @('schema', 'check', 'schema.sdl', '--bogus')
    Check ($code -eq 2 -and $err -match 'unknown option --bogus') 'an unknown option exits 2'
    Run @('ql', 'check', 'queries.ql')
    Check ($code -eq 2 -and $err -match '--schema') 'a missing --schema exits 2'
    Run @('schema', 'check', 'missing.sdl')
    Check ($code -eq 2 -and $err -match 'cannot read') 'a missing file exits 2'
    Run @('ql', 'compile', 'queries.ql', '--schema', 'schema.sdl', '--dialect', 'oracle')
    Check ($code -eq 2 -and $err -match 'unknown dialect') 'an unknown dialect exits 2'
}
finally {
    Pop-Location -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
if ($failures) { Write-Host "`n$failures check(s) FAILED"; exit 1 }
Write-Host "`nall checks passed"
