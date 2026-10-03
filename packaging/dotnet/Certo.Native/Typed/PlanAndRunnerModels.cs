// Typed models for migration plans, SQL lowering and the migration runner.

using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace Certo.Models
{
    /// <summary>Base of results that can fail outright: <see cref="Ok"/> false with an <see cref="Error"/>.</summary>
    public abstract class CertoResult
    {
        [JsonPropertyName("ok")] public bool Ok { get; init; }
        /// <summary>Set when the call itself failed (see <see cref="CertoError.Code"/>).</summary>
        [JsonPropertyName("error")] public CertoError? Error { get; init; }

        internal virtual CertoException? Failure(string what) =>
            Ok ? null : CertoException.From(Error, new List<CertoDiagnostic>(), null, what);
    }

    public static class CertoResultExtensions
    {
        /// <summary>Returns the result, or throws a <see cref="CertoException"/> if the call failed.</summary>
        public static T EnsureOk<T>(this T result, string what = "Certo call failed") where T : CertoResult =>
            result.Failure(what) is { } failure ? throw failure : result;
    }

    // ---- plans ------------------------------------------------------------------------------

    /// <summary>One operation of a migration plan, as a line of text.</summary>
    public sealed class PlanStep
    {
        /// <summary>For example <c>+ table users</c> (add), <c>~ column users.age</c> (change), <c>- table old</c> (remove).</summary>
        [JsonPropertyName("text")] public string Text { get; init; } = "";
        /// <summary>Can this step lose data or fail on existing rows?</summary>
        [JsonPropertyName("destructive")] public bool Destructive { get; init; }
    }

    /// <summary>The result of diffing two schemas (optionally steered by an MDL migration).</summary>
    public sealed class MigrationPlanResult : CertoResult
    {
        /// <summary>True when the two schemas are the same.</summary>
        [JsonPropertyName("empty")] public bool Empty { get; init; }
        [JsonPropertyName("destructive")] public bool Destructive { get; init; }
        [JsonPropertyName("summary")] public List<PlanStep> Summary { get; init; } = new();
        /// <summary>The plan, to pass to <see cref="CertoSql.Lower"/>.</summary>
        [JsonPropertyName("plan")] public JsonElement Plan { get; init; }
        /// <summary>Problems in the MDL source (positions refer to it); empty for a plain diff.</summary>
        [JsonPropertyName("diagnostics")] public List<CertoDiagnostic> Diagnostics { get; init; } = new();
        [JsonPropertyName("rendered")] public string? Rendered { get; init; }

        /// <summary>The plan as JSON text, for <see cref="CertoSql.Lower"/>.</summary>
        public string PlanJson => Plan.GetRawText();

        internal override CertoException? Failure(string what) =>
            Ok && Plan.ValueKind == JsonValueKind.Object ? null : CertoException.From(Error, Diagnostics, Rendered, what);
    }

    // ---- SQL ----------------------------------------------------------------------------------

    /// <summary>Statements to run together.</summary>
    public sealed class SqlBatch
    {
        /// <summary>Run all statements in one transaction; when false, each statement on its own (for example PostgreSQL enum <c>ADD VALUE</c>, or SQLite's foreign-key pragma).</summary>
        [JsonPropertyName("transactional")] public bool Transactional { get; init; }
        [JsonPropertyName("statements")] public List<string> Statements { get; init; } = new();
    }

    public sealed class LowerSqlResult : CertoResult
    {
        /// <summary>The batches to run in order.</summary>
        [JsonPropertyName("batches")] public List<SqlBatch> Batches { get; init; } = new();
        /// <summary>The same as one script, with BEGIN/COMMIT around the transactional batches.</summary>
        [JsonPropertyName("script")] public string? Script { get; init; }
    }

    // ---- migration runner --------------------------------------------------------------------

    public enum DriftKind { Missing, Unexpected, Different }

    /// <summary>One difference between the database and the migrations.</summary>
    public sealed class DriftItem
    {
        [JsonPropertyName("kind"), JsonConverter(typeof(LowerEnumConverter<DriftKind>))]
        public DriftKind Kind { get; init; }
        [JsonPropertyName("text")] public string Text { get; init; } = "";
        public override string ToString() => $"{Kind.ToString().ToLowerInvariant()}: {Text}";
    }

    public sealed class MigrateInitResult : CertoResult
    {
        [JsonPropertyName("root")] public string Root { get; init; } = "";
        [JsonPropertyName("dialect")] public string Dialect { get; init; } = "";
    }

    /// <summary>A migration frozen by <see cref="CertoMigrations.New"/>.</summary>
    public sealed class MigrationCreated : CertoResult
    {
        [JsonPropertyName("seq")] public int Seq { get; init; }
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        [JsonPropertyName("dir")] public string Dir { get; init; } = "";
        /// <summary>For example <c>0003_add_slug</c>.</summary>
        public string Label => $"{Seq:D4}_{Name}";
        /// <summary>One line per operation: <c>+</c> add, <c>~</c> change, <c>-</c> remove.</summary>
        [JsonPropertyName("summary")] public List<string> Summary { get; init; } = new();
        [JsonPropertyName("destructive")] public bool Destructive { get; init; }
        /// <summary>Rendered compiler warnings, if any.</summary>
        [JsonPropertyName("warnings")] public string Warnings { get; init; } = "";
    }

    /// <summary>A migration on disk.</summary>
    public sealed class MigrationInfo
    {
        [JsonPropertyName("seq")] public int Seq { get; init; }
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        /// <summary>For example <c>0003_add_slug</c>.</summary>
        [JsonPropertyName("label")] public string Label { get; init; } = "";
        [JsonPropertyName("dir")] public string Dir { get; init; } = "";
        [JsonPropertyName("checksum")] public string Checksum { get; init; } = "";
        [JsonPropertyName("batches")] public int Batches { get; init; }
        [JsonPropertyName("statements")] public int Statements { get; init; }
    }

    public sealed class MigrateListResult : CertoResult
    {
        [JsonPropertyName("migrations")] public List<MigrationInfo> Migrations { get; init; } = new();
    }

    public sealed class AppliedMigration
    {
        [JsonPropertyName("seq")] public int Seq { get; init; }
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        [JsonPropertyName("checksum")] public string Checksum { get; init; } = "";
        [JsonPropertyName("applied_at")] public string AppliedAt { get; init; } = "";
        /// <summary>For example <c>0003_add_slug</c>.</summary>
        public string Label => $"{Seq:D4}_{Name}";
    }

    public sealed class PendingMigration
    {
        [JsonPropertyName("seq")] public int Seq { get; init; }
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        /// <summary>For example <c>0003_add_slug</c>.</summary>
        public string Label => $"{Seq:D4}_{Name}";
    }

    public sealed class MigrateStatusResult : CertoResult
    {
        [JsonPropertyName("applied")] public List<AppliedMigration> Applied { get; init; } = new();
        [JsonPropertyName("pending")] public List<PendingMigration> Pending { get; init; } = new();
    }

    public sealed class MigrationScript
    {
        [JsonPropertyName("label")] public string Label { get; init; } = "";
        [JsonPropertyName("sql")] public string Sql { get; init; } = "";
    }

    public sealed class MigrateApplyResult : CertoResult
    {
        [JsonPropertyName("dry_run")] public bool DryRun { get; init; }
        /// <summary>Labels of the migrations applied, or that would be.</summary>
        [JsonPropertyName("migrations")] public List<string> Migrations { get; init; } = new();
        /// <summary>For a dry run: the SQL each pending migration would execute.</summary>
        [JsonPropertyName("scripts")] public List<MigrationScript> Scripts { get; init; } = new();
    }

    public sealed class MigrateDriftResult : CertoResult
    {
        [JsonPropertyName("in_sync")] public bool InSync { get; init; }
        /// <summary>What the database was compared with, for example <c>the schema after 0002_add_age</c>.</summary>
        [JsonPropertyName("expected_from")] public string ExpectedFrom { get; init; } = "";
        [JsonPropertyName("items")] public List<DriftItem> Items { get; init; } = new();
        /// <summary>Live objects the schema language cannot represent.</summary>
        [JsonPropertyName("notes")] public List<string> Notes { get; init; } = new();
        /// <summary>A script that would bring the database back to the migrations (review before running).</summary>
        [JsonPropertyName("repair_sql")] public string? RepairSql { get; init; }
        [JsonPropertyName("repair_error")] public string? RepairError { get; init; }
    }

    /// <summary>What an adoption took from the database.</summary>
    public sealed class AdoptedCounts
    {
        [JsonPropertyName("tables")] public int Tables { get; init; }
        [JsonPropertyName("columns")] public int Columns { get; init; }
        [JsonPropertyName("enums")] public int Enums { get; init; }
        [JsonPropertyName("types")] public int Types { get; init; }
        [JsonPropertyName("sequences")] public int Sequences { get; init; }
        [JsonPropertyName("indexes")] public int Indexes { get; init; }
        [JsonPropertyName("constraints")] public int Constraints { get; init; }
    }

    public sealed class MigrateAdoptResult : CertoResult
    {
        [JsonPropertyName("dry_run")] public bool DryRun { get; init; }
        /// <summary>The adopted schema, as SDL.</summary>
        [JsonPropertyName("schema_sdl")] public string SchemaSdl { get; init; } = "";
        [JsonPropertyName("adopted")] public AdoptedCounts Adopted { get; init; } = new();
        /// <summary>Everything in the database the schema language cannot express, and so was left out.</summary>
        [JsonPropertyName("omissions")] public List<string> Omissions { get; init; } = new();
        /// <summary>Label of the baseline migration (null for a dry run).</summary>
        [JsonPropertyName("migration")] public string? Migration { get; init; }
        /// <summary>Differences that remain between the database and the adopted schema.</summary>
        [JsonPropertyName("known_drift")] public List<DriftItem> KnownDrift { get; init; } = new();
    }
}
