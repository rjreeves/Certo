using System.Text.Json;
using Dapper;
using Npgsql;
using Oe.Application.Shared;
using Oe.Domain.Exceptions;

namespace Oe.Infrastructure.Validation;

/// <summary>
/// Calls the generated preflight_*(record JSONB) Postgres function.
/// The function returns JSONB: { "valid": bool, "violations": [...] }
/// Throws RuleViolationException if valid = false.
/// </summary>
public sealed class PostgresValidator(NpgsqlConnection connection, NpgsqlTransaction transaction)
    : IValidator
{
    public async Task ValidateAsync(string functionName, object record, CancellationToken ct = default)
    {
        var recordJson = JsonSerializer.Serialize(record, JsonOptions);

        var result = await connection.QuerySingleAsync<string>(
            $"SELECT {functionName}(@record::JSONB)::TEXT",
            new { record = recordJson },
            transaction);

        var doc = JsonDocument.Parse(result!);
        var root = doc.RootElement;

        if (root.GetProperty("valid").GetBoolean())
            return;

        var violations = root.GetProperty("violations")
            .EnumerateArray()
            .Select(v => new RuleViolation(
                v.GetProperty("code").GetString()!,
                v.GetProperty("message").GetString()!))
            .ToList();

        throw new RuleViolationException(violations);
    }

    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower
    };
}
