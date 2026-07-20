namespace Oe.Application.Shared;

/// <summary>
/// Calls the generated preflight_* Postgres function for an action.
/// Throws RuleViolationException if any rules fail.
/// Must be called inside an open transaction with the rule context already set.
/// </summary>
public interface IValidator
{
    Task ValidateAsync(string functionName, object record, CancellationToken ct = default);
}
