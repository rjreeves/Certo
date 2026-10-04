// Typed entry points for plans, SQL lowering and the migration runner.

using System.Collections.Generic;
using System.Text.Json;
using Certo.Models;

namespace Certo
{
    /// <summary>Diff schemas into migration plans.</summary>
    public static class CertoPlans
    {
        /// <summary>The plan that turns <paramref name="oldIrJson"/> into <paramref name="newIrJson"/> (schema IR as JSON text).</summary>
        public static MigrationPlanResult Diff(string oldIrJson, string newIrJson) =>
            Wire.Parse<MigrationPlanResult>(CertoNative.DiffIr(oldIrJson, newIrJson), "DiffIr");

        /// <summary>Like <see cref="Diff"/>, steered by an MDL migration (renames, enum remaps, backfills, before/after steps).</summary>
        public static MigrationPlanResult Plan(string oldIrJson, string newIrJson, string mdlSource) =>
            Wire.Parse<MigrationPlanResult>(CertoNative.PlanMigration(oldIrJson, newIrJson, mdlSource), "PlanMigration");
    }

    /// <summary>Lower plans to SQL.</summary>
    public static class CertoSql
    {
        /// <summary>
        /// Lower a plan to SQL batches. SQLite needs the schemas the plan was made between
        /// (<paramref name="oldIrJson"/> and <paramref name="newIrJson"/>), because it rebuilds tables;
        /// PostgreSQL ignores them.
        /// </summary>
        public static LowerSqlResult Lower(string planJson, SqlDialect dialect, string? oldIrJson = null, string? newIrJson = null) =>
            Wire.Parse<LowerSqlResult>(
                oldIrJson is not null && newIrJson is not null
                    ? CertoNative.LowerSqlWithSchemas(planJson, dialect.Name(), oldIrJson, newIrJson)
                    : CertoNative.LowerSql(planJson, dialect.Name()),
                "LowerSql");
    }

    /// <summary>Options for <see cref="CertoMigrations.New"/>.</summary>
    public sealed class NewMigrationOptions
    {
        /// <summary>The migration's name (required).</summary>
        public string Name { get; init; } = "";
        /// <summary>An MDL migration that steers the diff (renames, backfills, ...).</summary>
        public string? MdlSource { get; init; }
        public string? MdlLabel { get; init; }
        /// <summary>Allow operations that can lose data.</summary>
        public bool AllowDestructive { get; init; }
    }

    /// <summary>
    /// Who is changing the database, for the optional journal: with it, each applied migration (and an adopted baseline) also
    /// adds a row to the append-only table <c>_certo_log</c>, inside the transaction that makes the change, so the journal holds
    /// exactly the changes that committed.
    /// </summary>
    public sealed class JournalContext
    {
        /// <summary>The person or service, for example <c>alice@build-host</c>.</summary>
        public string Actor { get; init; } = "";
        /// <summary>The named environment, if the host has one.</summary>
        public string? Environment { get; init; }
        /// <summary>The tool and its version, for example <c>certo 0.10.0</c>.</summary>
        public string Tool { get; init; } = "";

        internal System.Collections.Generic.Dictionary<string, string?> ToWire() => new() { ["actor"] = Actor, ["environment"] = Environment, ["tool"] = Tool };
    }

    public sealed class ApplyOptions
    {
        /// <summary>The database: a <c>postgres://</c> URL, or a file path for a SQLite project.</summary>
        public string? Url { get; init; }
        /// <summary>Report what would run; touch nothing.</summary>
        public bool DryRun { get; init; }
        /// <summary>Stop after this migration number.</summary>
        public int? To { get; init; }
        /// <summary>Refuse to apply if the database has drifted from the last applied migration.</summary>
        public bool CheckDrift { get; init; }
        /// <summary>Write each applied migration to the journal (<c>_certo_log</c>), inside its transaction.</summary>
        public JournalContext? Journal { get; init; }
    }

    public sealed class DriftOptions
    {
        public string? Url { get; init; }
        /// <summary>Include the script that would repair the drift (default true).</summary>
        public bool RepairSql { get; init; } = true;
    }

    public sealed class AdoptOptions
    {
        public string? Url { get; init; }
        /// <summary>Report what would be adopted; write and record nothing.</summary>
        public bool DryRun { get; init; }
        /// <summary>Overwrite a <c>schema.sdl</c> that already has declarations.</summary>
        public bool Force { get; init; }
        /// <summary>Write the baseline to the journal (<c>_certo_log</c>), inside the transaction that records it.</summary>
        public JournalContext? Journal { get; init; }
    }

    /// <summary>
    /// The migration runner: a project directory holding <c>schema.sdl</c>, <c>IR.json</c> and the frozen
    /// migrations. Each call connects, works and disconnects. Calls with a database block until it answers:
    /// run them off the UI thread. A call that fails comes back with <see cref="CertoResult.Error"/>
    /// (<c>destructive</c>, <c>no_changes</c>, <c>compile</c>, <c>database</c>, <c>connection</c>, ...);
    /// <c>EnsureOk()</c> turns that into a <see cref="CertoException"/>.
    /// </summary>
    public static class CertoMigrations
    {
        private static string Json(Dictionary<string, object?> options)
        {
            var kept = new Dictionary<string, object?>();
            foreach (var kv in options) if (kv.Value is not null) kept[kv.Key] = kv.Value;
            return JsonSerializer.Serialize(kept);
        }

        /// <summary>Create a migration project in <paramref name="projectDir"/>.</summary>
        public static MigrateInitResult Init(string projectDir, SqlDialect dialect = SqlDialect.Postgres) =>
            Wire.Parse<MigrateInitResult>(
                CertoNative.MigrateInit(projectDir, Json(new() { ["dialect"] = dialect.Name() })), "MigrateInit");

        /// <summary>Freeze the difference between <c>schema.sdl</c> and <c>IR.json</c> as the next migration.</summary>
        public static MigrationCreated New(string projectDir, NewMigrationOptions options)
        {
            var o = new Dictionary<string, object?> { ["name"] = options.Name };
            if (options.AllowDestructive) o["allow_destructive"] = true;
            if (options.MdlSource is not null)
                o["mdl"] = new Dictionary<string, object?> { ["label"] = options.MdlLabel, ["source"] = options.MdlSource };
            return Wire.Parse<MigrationCreated>(CertoNative.MigrateNew(projectDir, Json(o)), "MigrateNew");
        }

        /// <summary>The migrations on disk (no database needed).</summary>
        public static MigrateListResult List(string projectDir) =>
            Wire.Parse<MigrateListResult>(CertoNative.MigrateList(projectDir), "MigrateList");

        /// <summary>Applied and pending migrations.</summary>
        public static MigrateStatusResult Status(string projectDir, string url) =>
            Wire.Parse<MigrateStatusResult>(CertoNative.MigrateStatus(projectDir, Json(new() { ["url"] = url })), "MigrateStatus");

        /// <summary>Apply pending migrations in order. Stops at the first failure; earlier ones stay applied (<c>Error.Applied</c> lists them).</summary>
        public static MigrateApplyResult Apply(string projectDir, ApplyOptions options) =>
            Wire.Parse<MigrateApplyResult>(
                CertoNative.MigrateApply(projectDir, Json(new()
                {
                    ["url"] = options.Url,
                    ["dry_run"] = options.DryRun ? true : null,
                    ["to"] = options.To,
                    ["check_drift"] = options.CheckDrift ? true : null,
                    ["journal"] = options.Journal?.ToWire(),
                })),
                "MigrateApply");

        /// <summary>Compare the database with the last applied migration.</summary>
        public static MigrateDriftResult Drift(string projectDir, DriftOptions options) =>
            Wire.Parse<MigrateDriftResult>(
                CertoNative.MigrateDrift(projectDir, Json(new() { ["url"] = options.Url, ["repair_sql"] = options.RepairSql })),
                "MigrateDrift");

        /// <summary>Adopt an existing database into a fresh project: writes <c>schema.sdl</c> and records a baseline migration as applied (never run).</summary>
        public static MigrateAdoptResult Adopt(string projectDir, AdoptOptions options) =>
            Wire.Parse<MigrateAdoptResult>(
                CertoNative.MigrateAdopt(projectDir, Json(new()
                {
                    ["url"] = options.Url,
                    ["dry_run"] = options.DryRun ? true : null,
                    ["force"] = options.Force ? true : null,
                    ["journal"] = options.Journal?.ToWire(),
                })),
                "MigrateAdopt");
    }
}
