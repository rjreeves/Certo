using System.Text.RegularExpressions;
namespace RuleGenerator.Validators;

/// <summary>
/// Validates business rules for Invoice.void.
/// <list type="bullet">
///   <item>invoice_void_admin_override — Admin users can void invoices outside the standard void window</item>
///   <item>invoice_void_within_window — An invoice can only be voided within the standard void window</item>
/// </list>
/// Generated — do not edit.
/// </summary>
public sealed class InvoiceVoidValidator : IValidator<InvoiceContext>
{
    public void Validate(InvoiceContext context)
    {
        // Rule suspended when 'invoice_void_admin_override' condition passes
        if (!(context?.User.Role == "ADMIN"))
        {
            // An invoice can only be voided within the standard void window
            if (!Check_InvoiceVoidWithinWindow_Condition(context))
            {
                throw new RuleViolationException(
                    "INVOICE_VOID_WINDOW_EXPIRED",
                    "Invoices cannot be voided after 30 days or once paid");
            }
        }
    }

    public IReadOnlyList<RuleViolationException> ValidateAll(InvoiceContext context)
    {
        var violations = new List<RuleViolationException>();
        // Rule suspended when 'invoice_void_admin_override' condition passes
        if (!(context?.User.Role == "ADMIN"))
        {
            // An invoice can only be voided within the standard void window
            if (!Check_InvoiceVoidWithinWindow_Condition(context))
            {
                violations.Add(new RuleViolationException(
                    "INVOICE_VOID_WINDOW_EXPIRED",
                    "Invoices cannot be voided after 30 days or once paid"));
            }
        }
        return violations;
    }

    /// <summary>Admin users can void invoices outside the standard void window</summary>
    private static bool Check_InvoiceVoidAdminOverride_Condition(InvoiceContext context)
    {
        return context?.User.Role == "ADMIN";
    }

    /// <summary>An invoice can only be voided within the standard void window</summary>
    private static bool Check_InvoiceVoidWithinWindow_Condition(InvoiceContext context)
    {
        return ((DateTimeOffset.UtcNow - (context?.Invoice.CreatedAt ?? DateTimeOffset.MinValue)).TotalDays < 30)
            && (!new[] { "PAID", "WRITTEN_OFF" }.Contains(context?.Invoice.Status));
    }

}