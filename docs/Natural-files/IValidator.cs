namespace RuleGenerator.Validators;

/// <summary>Common validator interface. Generated — do not edit.</summary>
public interface IValidator<TContext>
{
    /// <summary>Throws RuleViolationException on first violation.</summary>
    void Validate(TContext context);

    /// <summary>Returns all violations without stopping at first.</summary>
    IReadOnlyList<RuleViolationException> ValidateAll(TContext context);
}
