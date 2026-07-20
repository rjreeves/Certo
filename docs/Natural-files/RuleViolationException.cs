namespace RuleGenerator.Validators;

/// <summary>Thrown when a business rule is violated. Generated — do not edit.</summary>
public sealed class RuleViolationException : Exception
{
    public string RuleCode    { get; }
    public string UserMessage { get; }

    public RuleViolationException(string ruleCode, string userMessage)
        : base($"[{ruleCode}] {userMessage}")
    {
        RuleCode    = ruleCode;
        UserMessage = userMessage;
    }
}

/// <summary>Aggregates multiple violations. Generated — do not edit.</summary>
public sealed class AggregateRuleViolationException : Exception
{
    public IReadOnlyList<RuleViolationException> Violations { get; }

    public AggregateRuleViolationException(IEnumerable<RuleViolationException> violations)
        : base("One or more business rules were violated.")
    {
        Violations = violations.ToList();
    }
}
