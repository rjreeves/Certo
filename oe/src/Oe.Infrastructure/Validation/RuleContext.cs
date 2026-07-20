using Dapper;
using Npgsql;
using Oe.Domain.Enums;

namespace Oe.Infrastructure.Validation;

/// <summary>
/// Sets the session-local rule context variables that current_rule_context()
/// reads inside validator functions.  Must be called once per transaction
/// before any preflight or DML.
/// </summary>
public static class RuleContext
{
    public static async Task SetAsync(
        NpgsqlConnection  connection,
        NpgsqlTransaction transaction,
        Guid              userId,
        UserRole          role,
        CancellationToken ct = default)
    {
        var roleStr = role.ToString().ToUpperInvariant();

        await connection.ExecuteAsync(
            """
            SET LOCAL rule.user_id   = @userId;
            SET LOCAL rule.user_role = @role;
            """,
            new { userId = userId.ToString(), role = roleStr },
            transaction);
    }
}
