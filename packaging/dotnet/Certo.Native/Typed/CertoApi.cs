// Typed entry points over CertoNative: the same calls, with models in and out.

using System;
using System.Text.Json;
using Certo.Models;

namespace Certo
{
    public enum SqlDialect { Postgres, Sqlite }

    internal static class Wire
    {
        internal static readonly JsonSerializerOptions Options = new() { PropertyNameCaseInsensitive = false };

        internal static string Name(this SqlDialect d) => d switch
        {
            SqlDialect.Postgres => "postgres",
            SqlDialect.Sqlite => "sqlite",
            _ => throw new ArgumentOutOfRangeException(nameof(d)),
        };

        internal static T Parse<T>(string json, string call)
        {
            try
            {
                return JsonSerializer.Deserialize<T>(json, Options)
                    ?? throw new CertoException($"{call} returned nothing");
            }
            catch (JsonException e)
            {
                throw new CertoException($"{call} returned JSON this version of Certo.Native cannot read: {e.Message}");
            }
        }
    }

    /// <summary>Compile SDL schemas.</summary>
    public static class CertoSdl
    {
        /// <summary>Compile schema source. Errors are in <see cref="SdlCompileResult.Diagnostics"/>; use <see cref="SdlCompileResult.EnsureOk"/> to get the IR or an exception.</summary>
        public static SdlCompileResult Compile(string source) =>
            Wire.Parse<SdlCompileResult>(CertoNative.CompileSdl(source), "CompileSdl");

        /// <summary>
        /// Read a database's schema as SDL, with no project and without changing the database (a missing SQLite file is an
        /// error, not an empty database). <paramref name="url"/> is a <c>postgres://</c> URL or a SQLite file path; the dialect
        /// follows from it unless given. What SDL cannot express is listed in <see cref="SchemaImportResult.Omissions"/>.
        /// </summary>
        public static SchemaImportResult Import(string url, SqlDialect? dialect = null)
        {
            var options = new System.Collections.Generic.Dictionary<string, string> { ["url"] = url };
            if (dialect is { } d) options["dialect"] = d.Name();
            return Wire.Parse<SchemaImportResult>(CertoNative.SchemaImport(System.Text.Json.JsonSerializer.Serialize(options)), "SchemaImport");
        }
    }

    /// <summary>Compile QL queries and mutations, and generate host code from them.</summary>
    public static class CertoQl
    {
        /// <summary>Compile QL against a schema IR (the <c>Ir</c> of an <see cref="SdlCompileResult"/>, as JSON text).</summary>
        public static QlCompileResult Compile(string schemaIrJson, string qlSource, SqlDialect dialect = SqlDialect.Postgres) =>
            Wire.Parse<QlCompileResult>(CertoNative.CompileQl(schemaIrJson, qlSource, dialect.Name()), "CompileQl");

        /// <summary>Compile QL against a compiled schema; throws <see cref="CertoException"/> if the schema had errors.</summary>
        public static QlCompileResult Compile(SdlCompileResult schema, string qlSource, SqlDialect dialect = SqlDialect.Postgres) =>
            Compile(schema.EnsureOk(), qlSource, dialect);

        /// <summary>Generate a C# source file (records and ADO.NET extension methods) from QL.</summary>
        public static QlCodegenResult GenerateCSharp(
            string schemaIrJson, string qlSource, SqlDialect dialect = SqlDialect.Postgres,
            string? @namespace = null, string? className = null)
        {
            var options = new System.Collections.Generic.Dictionary<string, string>
            {
                ["language"] = "csharp",
                ["dialect"] = dialect.Name(),
            };
            if (@namespace is not null) options["namespace"] = @namespace;
            if (className is not null) options["class_name"] = className;
            return Wire.Parse<QlCodegenResult>(
                CertoNative.CodegenQl(schemaIrJson, qlSource, JsonSerializer.Serialize(options)), "CodegenQl");
        }
    }
}
