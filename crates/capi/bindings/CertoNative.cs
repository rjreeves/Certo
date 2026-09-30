// P/Invoke binding for certo_capi (see ../include/certo.h).
//
// Every method returns the raw JSON text of the result envelope; parse it with
// System.Text.Json. "ok": false is a normal outcome (schema errors); only the
// ABI-version check below throws.
//
// Place certo_capi.dll (or libcerto_capi.so / .dylib) next to the executable.

using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Certo;

public static class CertoNative
{
    /// <summary>The ABI this binding was written against.</summary>
    public const uint ExpectedAbiVersion = 1;

    private const string Lib = "certo_capi";

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern uint certo_abi_version();

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_version();

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_sdl_compile(byte[] source);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_diff_ir(byte[] oldIr, byte[] newIr);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_plan_migration(byte[] oldIr, byte[] newIr, byte[] mdl);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_ql_compile(byte[] schemaIr, byte[] qlSource, byte[] dialect);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_lower_sql(byte[] plan, byte[] dialect);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_lower_sql_with_schemas(byte[] plan, byte[] dialect, byte[] oldIr, byte[] newIr);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_init(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_new(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_list(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_status(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_apply(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_drift(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern IntPtr certo_migrate_adopt(byte[] dir, byte[]? options);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    private static extern void certo_string_free(IntPtr s);

    static CertoNative()
    {
        uint actual = certo_abi_version();
        if (actual != ExpectedAbiVersion)
            throw new InvalidOperationException(
                $"certo_capi ABI mismatch: library reports {actual}, binding expects {ExpectedAbiVersion}.");
    }

    public static string Version() => Take(certo_version());

    /// <summary>Compile SDL source. Result has ok, ir, diagnostics, rendered.</summary>
    public static string CompileSdl(string source) => Take(certo_sdl_compile(Z(source)));

    /// <summary>Diff two SchemaIR JSON documents into a migration plan.</summary>
    public static string DiffIr(string oldIrJson, string newIrJson) =>
        Take(certo_diff_ir(Z(oldIrJson), Z(newIrJson)));

    /// <summary>
    /// Diff steered by an MDL migration (rename / remap / backfill / before / after).
    /// MDL problems are returned as diagnostics positioned in the MDL text.
    /// </summary>
    public static string PlanMigration(string oldIrJson, string newIrJson, string mdlSource) =>
        Take(certo_plan_migration(Z(oldIrJson), Z(newIrJson), Z(mdlSource)));

    /// <summary>
    /// Compile QL (queries, insert, update, delete) against a schema (the "ir" from CompileSdl). Each statement comes back
    /// as a typed contract: parameters, result columns with type and nullability, the SQL, and
    /// param_order (which declared parameter is $1, $2, ...). Errors are positioned diagnostics.
    /// </summary>
    public static string CompileQl(string schemaIrJson, string qlSource, string dialect = "postgres") =>
        Take(certo_ql_compile(Z(schemaIrJson), Z(qlSource), Z(dialect)));

    /// <summary>Lower a plan JSON document to SQL batches ("postgres"; "sqlite" needs <see cref="LowerSqlWithSchemas"/>).</summary>
    public static string LowerSql(string planJson, string dialect) =>
        Take(certo_lower_sql(Z(planJson), Z(dialect)));

    /// <summary>
    /// Lower a plan with the old and new schema IR it was made between. Required for "sqlite",
    /// which rebuilds tables (PRAGMA foreign_keys OFF, the transaction, PRAGMA foreign_keys ON).
    /// </summary>
    public static string LowerSqlWithSchemas(string planJson, string dialect, string oldIrJson, string newIrJson) =>
        Take(certo_lower_sql_with_schemas(Z(planJson), Z(dialect), Z(oldIrJson), Z(newIrJson)));

    // ---- migration runner ------------------------------------------------
    // Options are JSON objects (or null). Every call returns the result JSON;
    // {"ok": false, "error": {"code", ...}} is a normal outcome, not an exception.
    // Database calls block until the database answers: run them off the UI thread.

    /// <summary>Create a migration project. Options: {"dialect":"postgres"}.</summary>
    public static string MigrateInit(string projectDir, string? optionsJson = null) =>
        Take(certo_migrate_init(Z(projectDir), ZN(optionsJson)));

    /// <summary>Freeze schema.sdl vs IR.json as the next migration. Options: {"name", "mdl":{"label","source"}?, "allow_destructive"?}.</summary>
    public static string MigrateNew(string projectDir, string optionsJson) =>
        Take(certo_migrate_new(Z(projectDir), ZN(optionsJson)));

    /// <summary>List migrations on disk (no database).</summary>
    public static string MigrateList(string projectDir) =>
        Take(certo_migrate_list(Z(projectDir), null));

    /// <summary>Applied vs pending. Options: {"url"}.</summary>
    public static string MigrateStatus(string projectDir, string optionsJson) =>
        Take(certo_migrate_status(Z(projectDir), ZN(optionsJson)));

    /// <summary>Apply pending migrations. Options: {"url", "dry_run"?, "to"?, "check_drift"?}.</summary>
    public static string MigrateApply(string projectDir, string optionsJson) =>
        Take(certo_migrate_apply(Z(projectDir), ZN(optionsJson)));

    /// <summary>Compare the live database with the last applied migration. Options: {"url", "repair_sql"?}.</summary>
    public static string MigrateDrift(string projectDir, string optionsJson) =>
        Take(certo_migrate_drift(Z(projectDir), ZN(optionsJson)));

    /// <summary>
    /// Adopt an existing database into a fresh project: writes schema.sdl and records a
    /// baseline migration as applied (not run). Options: {"url", "dry_run"?, "force"?}.
    /// The result lists omissions (what SDL cannot express) and known_drift.
    /// </summary>
    public static string MigrateAdopt(string projectDir, string optionsJson) =>
        Take(certo_migrate_adopt(Z(projectDir), ZN(optionsJson)));

    private static byte[]? ZN(string? s) => s is null ? null : Z(s);

    // UTF-8, NUL-terminated.
    private static byte[] Z(string s)
    {
        var bytes = new byte[Encoding.UTF8.GetByteCount(s) + 1];
        Encoding.UTF8.GetBytes(s, 0, s.Length, bytes, 0);
        return bytes;
    }

    // Copy the Rust-owned string into managed memory, then hand it back.
    private static string Take(IntPtr p)
    {
        if (p == IntPtr.Zero) throw new InvalidOperationException("certo_capi returned NULL");
        try { return Marshal.PtrToStringUTF8(p)!; }
        finally { certo_string_free(p); }
    }
}
