namespace Oe.Domain.Exceptions;

public sealed record RuleViolation(string Code, string Message);

/// <summary>
/// Thrown when one or more business rule preflight checks fail.
/// Maps to HTTP 422 Unprocessable Entity.
/// </summary>
public sealed class RuleViolationException(IReadOnlyList<RuleViolation> violations)
    : Exception("One or more business rules were violated.")
{
    public IReadOnlyList<RuleViolation> Violations { get; } = violations;

    public static RuleViolationException Single(string code, string message) =>
        new([new RuleViolation(code, message)]);
}
