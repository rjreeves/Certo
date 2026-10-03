// Typed models for the JSON the native library returns, so hosts do not parse it by hand.
// Shapes mirror crates/capi/src/api.rs; unknown members are ignored, so the library can add
// fields without breaking a host built against an older package.

using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

#if !NET5_0_OR_GREATER
namespace System.Runtime.CompilerServices { internal static class IsExternalInit { } }
#endif

namespace Certo.Models
{
    /// <summary>Why a call failed outright (as opposed to the source having errors, which are diagnostics).</summary>
    public sealed class CertoError
    {
        /// <summary>A stable code such as <c>invalid_ir</c>, <c>unknown_dialect</c> or <c>invalid_options</c>.</summary>
        [JsonPropertyName("code")] public string Code { get; init; } = "";
        [JsonPropertyName("message")] public string Message { get; init; } = "";
    }

    public enum DiagnosticSeverity { Error, Warning, Note }

    /// <summary>A position in the source text. Lines and columns are 1-based; columns count UTF-16 code units, like .NET strings.</summary>
    public sealed class SourceSpan
    {
        [JsonPropertyName("start")] public int Start { get; init; }
        [JsonPropertyName("end")] public int End { get; init; }
        [JsonPropertyName("line")] public int Line { get; init; }
        [JsonPropertyName("column")] public int Column { get; init; }
        [JsonPropertyName("end_line")] public int EndLine { get; init; }
        [JsonPropertyName("end_column")] public int EndColumn { get; init; }
    }

    /// <summary>An error or warning in a schema or query, positioned in the source.</summary>
    public sealed class CertoDiagnostic
    {
        [JsonPropertyName("severity"), JsonConverter(typeof(LowerEnumConverter<DiagnosticSeverity>))]
        public DiagnosticSeverity Severity { get; init; }
        /// <summary>For example <c>QL206</c> (unknown column) or <c>SDL100</c> (syntax).</summary>
        [JsonPropertyName("code")] public string Code { get; init; } = "";
        [JsonPropertyName("message")] public string Message { get; init; } = "";
        [JsonPropertyName("span")] public SourceSpan? Span { get; init; }
        [JsonPropertyName("label")] public string? Label { get; init; }
        [JsonPropertyName("notes")] public List<string> Notes { get; init; } = new();

        public override string ToString() =>
            Span is null ? $"{Severity.ToString().ToLowerInvariant()} {Code}: {Message}"
                         : $"{Severity.ToString().ToLowerInvariant()} {Code} at {Span.Line}:{Span.Column}: {Message}";
    }

    /// <summary>A type from the schema: a builtin (with its length or precision), an enum, or a composite.</summary>
    [JsonConverter(typeof(SchemaTypeConverter))]
    public sealed class SchemaType
    {
        /// <summary><c>builtin</c>, <c>enum</c> or <c>composite</c>.</summary>
        public string Kind { get; init; } = "builtin";
        /// <summary>The builtin's name (<c>int</c>, <c>text</c>, <c>decimal</c>, <c>varchar</c>, <c>timestamp_naive</c>, ...) or the enum / composite's.</summary>
        public string Name { get; init; } = "";
        /// <summary>The <c>n</c> of <c>varchar(n)</c> / <c>char(n)</c>.</summary>
        public int? Length { get; init; }
        public int? Precision { get; init; }
        public int? Scale { get; init; }

        public bool IsEnum => Kind == "enum";
        public bool IsBuiltin => Kind == "builtin";

        /// <summary>The type as written in SDL: <c>int</c>, <c>varchar(100)</c>, <c>decimal(10,2)</c>, <c>Role</c>.</summary>
        public override string ToString() =>
            Length is { } n ? $"{Name}({n})"
            : Precision is { } p ? $"decimal({p},{Scale ?? 0})"
            : Name;
    }

    public enum StatementKind { Query, Insert, Update, Delete }

    public sealed class QlParam
    {
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        [JsonPropertyName("type")] public SchemaType Type { get; init; } = new();
        /// <summary>Declared <c>null</c>: the caller may pass NULL.</summary>
        [JsonPropertyName("nullable")] public bool Nullable { get; init; }
    }

    /// <summary>A result column: its type and whether it can be NULL in a row.</summary>
    public sealed class QlColumn
    {
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        [JsonPropertyName("type")] public SchemaType Type { get; init; } = new();
        [JsonPropertyName("nullable")] public bool Nullable { get; init; }
    }

    /// <summary>One compiled QL statement: its typed contract and the SQL.</summary>
    public sealed class QlStatement
    {
        [JsonPropertyName("kind"), JsonConverter(typeof(LowerEnumConverter<StatementKind>))]
        public StatementKind Kind { get; init; }
        [JsonPropertyName("name")] public string Name { get; init; } = "";
        /// <summary>Declared parameters, in declaration order.</summary>
        [JsonPropertyName("params")] public List<QlParam> Params { get; init; } = new();
        /// <summary>Result columns (a mutation's are its <c>returning</c> list; empty means a row count).</summary>
        [JsonPropertyName("columns")] public List<QlColumn> Columns { get; init; } = new();
        [JsonPropertyName("sql")] public string Sql { get; init; } = "";
        /// <summary><c>ParamOrder[i]</c> is the declared parameter bound to placeholder <c>$(i+1)</c> (PostgreSQL) or <c>?(i+1)</c> (SQLite).</summary>
        [JsonPropertyName("param_order")] public List<string> ParamOrder { get; init; } = new();
        /// <summary>The full checked IR (expressions, sources, ...), for tooling that needs more than the contract.</summary>
        [JsonPropertyName("ir")] public JsonElement Ir { get; init; }

        public bool IsMutation => Kind != StatementKind.Query;
        public bool ReturnsRows => Columns.Count > 0;
    }

    /// <summary>The result of compiling a QL file.</summary>
    public sealed class QlCompileResult
    {
        /// <summary>True when there are no errors (warnings may accompany a success).</summary>
        [JsonPropertyName("ok")] public bool Ok { get; init; }
        /// <summary>Every statement, queries first and then mutations; null if <see cref="Ok"/> is false.</summary>
        [JsonPropertyName("statements")] public List<QlStatement>? Statements { get; init; }
        /// <summary>Just the read queries (kept for older hosts).</summary>
        [JsonPropertyName("queries")] public List<QlStatement>? Queries { get; init; }
        [JsonPropertyName("diagnostics")] public List<CertoDiagnostic> Diagnostics { get; init; } = new();
        /// <summary>The diagnostics as readable text with source excerpts.</summary>
        [JsonPropertyName("rendered")] public string? Rendered { get; init; }
        /// <summary>Set when the call itself failed (bad IR, unknown dialect); the source's own errors are <see cref="Diagnostics"/>.</summary>
        [JsonPropertyName("error")] public CertoError? Error { get; init; }

        /// <summary>The statements, or a <see cref="CertoException"/> carrying the diagnostics.</summary>
        public IReadOnlyList<QlStatement> EnsureOk()
        {
            if (!Ok || Statements is null) throw CertoException.From(Error, Diagnostics, Rendered, "QL compilation failed");
            return Statements;
        }
    }

    /// <summary>The result of generating host code from QL.</summary>
    public sealed class QlCodegenResult
    {
        [JsonPropertyName("ok")] public bool Ok { get; init; }
        /// <summary>One source file; null if <see cref="Ok"/> is false.</summary>
        [JsonPropertyName("code")] public string? Code { get; init; }
        [JsonPropertyName("diagnostics")] public List<CertoDiagnostic> Diagnostics { get; init; } = new();
        [JsonPropertyName("rendered")] public string? Rendered { get; init; }
        [JsonPropertyName("error")] public CertoError? Error { get; init; }

        public string EnsureOk()
        {
            if (!Ok || Code is null) throw CertoException.From(Error, Diagnostics, Rendered, "code generation failed");
            return Code;
        }
    }

    /// <summary>The result of compiling an SDL schema.</summary>
    public sealed class SdlCompileResult
    {
        [JsonPropertyName("ok")] public bool Ok { get; init; }
        /// <summary>The schema IR (canonical JSON), to persist as IR.json and to pass to QL; absent if <see cref="Ok"/> is false.</summary>
        [JsonPropertyName("ir")] public JsonElement Ir { get; init; }
        [JsonPropertyName("diagnostics")] public List<CertoDiagnostic> Diagnostics { get; init; } = new();
        [JsonPropertyName("rendered")] public string? Rendered { get; init; }
        [JsonPropertyName("error")] public CertoError? Error { get; init; }

        /// <summary>The IR as JSON text, or a <see cref="CertoException"/> carrying the diagnostics.</summary>
        public string EnsureOk()
        {
            if (!Ok || Ir.ValueKind != JsonValueKind.Object) throw CertoException.From(Error, Diagnostics, Rendered, "schema compilation failed");
            return Ir.GetRawText();
        }
    }

    /// <summary>A call failed, or its source had errors. <see cref="Diagnostics"/> has the positioned details.</summary>
    public sealed class CertoException : Exception
    {
        public IReadOnlyList<CertoDiagnostic> Diagnostics { get; }
        public CertoError? Error { get; }

        public CertoException(string message, CertoError? error = null, IReadOnlyList<CertoDiagnostic>? diagnostics = null)
            : base(message)
        {
            Error = error;
            Diagnostics = diagnostics ?? Array.Empty<CertoDiagnostic>();
        }

        internal static CertoException From(CertoError? error, List<CertoDiagnostic> diagnostics, string? rendered, string what)
        {
            var detail = error is not null ? $"{error.Code}: {error.Message}"
                       : !string.IsNullOrWhiteSpace(rendered) ? rendered!.TrimEnd()
                       : string.Join("; ", diagnostics);
            return new CertoException($"{what}: {detail}", error, diagnostics);
        }
    }

    // ---- JSON converters -------------------------------------------------------------------

    /// <summary>Enums spelled in lower case on the wire (<c>query</c>, <c>warning</c>).</summary>
    internal sealed class LowerEnumConverter<T> : JsonConverter<T> where T : struct, Enum
    {
        public override T Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            var s = reader.GetString();
            if (s is not null && Enum.TryParse<T>(s, ignoreCase: true, out var v)) return v;
            throw new JsonException($"unknown {typeof(T).Name} `{s}`");
        }

        public override void Write(Utf8JsonWriter writer, T value, JsonSerializerOptions options) =>
            writer.WriteStringValue(value.ToString().ToLowerInvariant());
    }

    /// <summary>
    /// Reads <c>{"kind":"builtin","name":"int"}</c>, <c>{"kind":"builtin","name":{"varchar":100}}</c>,
    /// <c>{"kind":"builtin","name":{"numeric":[10,2]}}</c> and <c>{"kind":"enum","name":"Role"}</c>.
    /// </summary>
    internal sealed class SchemaTypeConverter : JsonConverter<SchemaType>
    {
        public override SchemaType Read(ref Utf8JsonReader reader, Type typeToConvert, JsonSerializerOptions options)
        {
            using var doc = JsonDocument.ParseValue(ref reader);
            var root = doc.RootElement;
            var kind = root.TryGetProperty("kind", out var k) ? k.GetString() ?? "builtin" : "builtin";
            if (!root.TryGetProperty("name", out var name)) throw new JsonException("a type needs a name");
            switch (name.ValueKind)
            {
                case JsonValueKind.String:
                    return new SchemaType { Kind = kind, Name = name.GetString()! };
                case JsonValueKind.Object:
                    foreach (var p in name.EnumerateObject())
                    {
                        if (p.Name == "numeric" && p.Value.ValueKind == JsonValueKind.Array && p.Value.GetArrayLength() == 2)
                            return new SchemaType { Kind = kind, Name = "decimal", Precision = p.Value[0].GetInt32(), Scale = p.Value[1].GetInt32() };
                        if (p.Value.ValueKind == JsonValueKind.Number)
                            return new SchemaType { Kind = kind, Name = p.Name, Length = p.Value.GetInt32() };
                    }
                    break;
            }
            throw new JsonException($"unrecognised type: {root.GetRawText()}");
        }

        public override void Write(Utf8JsonWriter writer, SchemaType value, JsonSerializerOptions options)
        {
            writer.WriteStartObject();
            writer.WriteString("kind", value.Kind);
            writer.WritePropertyName("name");
            if (value.Length is { } n)
            {
                writer.WriteStartObject();
                writer.WriteNumber(value.Name, n);
                writer.WriteEndObject();
            }
            else if (value.Precision is { } p)
            {
                writer.WriteStartObject();
                writer.WritePropertyName("numeric");
                writer.WriteStartArray();
                writer.WriteNumberValue(p);
                writer.WriteNumberValue(value.Scale ?? 0);
                writer.WriteEndArray();
                writer.WriteEndObject();
            }
            else writer.WriteStringValue(value.Name);
            writer.WriteEndObject();
        }
    }
}
