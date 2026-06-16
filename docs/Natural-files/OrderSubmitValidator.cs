using System.Text.RegularExpressions;
namespace RuleGenerator.Validators;

/// <summary>
/// Validates business rules for Order.submit.
/// <list type="bullet">
///   <item>credit_limit_override_requires_admin — Submitting an order that exceeds the credit limit requires admin approval</item>
///   <item>customer_must_be_active_to_order — Orders can only be placed for customers with ACTIVE status</item>
///   <item>order_must_have_lines_to_submit — An order must have at least one line before it can be submitted</item>
///   <item>order_must_have_shipping_address — A physical order must have a shipping address before submission</item>
///   <item>order_must_have_billing_address — An order must have a billing address before submission</item>
///   <item>customer_credit_limit_not_exceeded — A submitted order must not cause the customer's outstanding balance to exceed their credit limit
</item>
/// </list>
/// Generated — do not edit.
/// </summary>
public sealed class OrderSubmitValidator : IValidator<OrderContext>
{
    public void Validate(OrderContext context)
    {
        // Orders can only be placed for customers with ACTIVE status
        if (!Check_CustomerMustBeActiveToOrder_Condition(context))
        {
            throw new RuleViolationException(
                "CUSTOMER_NOT_ACTIVE",
                "Orders cannot be placed for customers who are not active");
        }
        // An order must have at least one line before it can be submitted
        if (!Check_OrderMustHaveLinesToSubmit_Condition(context))
        {
            throw new RuleViolationException(
                "ORDER_HAS_NO_LINES",
                "An order must have at least one product line before submission");
        }
        // A physical order must have a shipping address before submission
        if (!Check_OrderMustHaveShippingAddress_Condition(context))
        {
            throw new RuleViolationException(
                "NO_SHIPPING_ADDRESS",
                "A shipping address is required before submitting this order");
        }
        // An order must have a billing address before submission
        if (!Check_OrderMustHaveBillingAddress_Condition(context))
        {
            throw new RuleViolationException(
                "NO_BILLING_ADDRESS",
                "A billing address is required before submitting this order");
        }
        // Rule suspended when 'credit_limit_override_requires_admin' condition passes
        if (!(context?.User.Role == "ADMIN"))
        {
            // Depends on: customer_must_be_active_to_order
            if (Check_CustomerMustBeActiveToOrder_Condition(context))
            {
                // A submitted order must not cause the customer's outstanding balance to exceed their credit limit

                if (!Check_CustomerCreditLimitNotExceeded_Condition(context))
                {
                    throw new RuleViolationException(
                        "CREDIT_LIMIT_EXCEEDED",
                        "This order would exceed the customer's available credit limit");
                }
            }
        }
    }

    public IReadOnlyList<RuleViolationException> ValidateAll(OrderContext context)
    {
        var violations = new List<RuleViolationException>();
        // Orders can only be placed for customers with ACTIVE status
        if (!Check_CustomerMustBeActiveToOrder_Condition(context))
        {
            violations.Add(new RuleViolationException(
                "CUSTOMER_NOT_ACTIVE",
                "Orders cannot be placed for customers who are not active"));
        }
        // An order must have at least one line before it can be submitted
        if (!Check_OrderMustHaveLinesToSubmit_Condition(context))
        {
            violations.Add(new RuleViolationException(
                "ORDER_HAS_NO_LINES",
                "An order must have at least one product line before submission"));
        }
        // A physical order must have a shipping address before submission
        if (!Check_OrderMustHaveShippingAddress_Condition(context))
        {
            violations.Add(new RuleViolationException(
                "NO_SHIPPING_ADDRESS",
                "A shipping address is required before submitting this order"));
        }
        // An order must have a billing address before submission
        if (!Check_OrderMustHaveBillingAddress_Condition(context))
        {
            violations.Add(new RuleViolationException(
                "NO_BILLING_ADDRESS",
                "A billing address is required before submitting this order"));
        }
        // Rule suspended when 'credit_limit_override_requires_admin' condition passes
        if (!(context?.User.Role == "ADMIN"))
        {
            // Depends on: customer_must_be_active_to_order
            if (Check_CustomerMustBeActiveToOrder_Condition(context))
            {
                // A submitted order must not cause the customer's outstanding balance to exceed their credit limit

                if (!Check_CustomerCreditLimitNotExceeded_Condition(context))
                {
                    violations.Add(new RuleViolationException(
                        "CREDIT_LIMIT_EXCEEDED",
                        "This order would exceed the customer's available credit limit"));
                }
            }
        }
        return violations;
    }

    /// <summary>Submitting an order that exceeds the credit limit requires admin approval</summary>
    private static bool Check_CreditLimitOverrideRequiresAdmin_Condition(OrderContext context)
    {
        return context?.User.Role == "ADMIN";
    }

    /// <summary>Orders can only be placed for customers with ACTIVE status</summary>
    private static bool Check_CustomerMustBeActiveToOrder_Condition(OrderContext context)
    {
        return context?.Customer.Status == "ACTIVE";
    }

    /// <summary>An order must have at least one line before it can be submitted</summary>
    private static bool Check_OrderMustHaveLinesToSubmit_Condition(OrderContext context)
    {
        return context?.Order.LineCount > 0;
    }

    /// <summary>A physical order must have a shipping address before submission</summary>
    private static bool Check_OrderMustHaveShippingAddress_Condition(OrderContext context)
    {
        return (context?.Order.ShippingAddressId != null)
            || (context?.Order.LinesAllDigital == true);
    }

    /// <summary>An order must have a billing address before submission</summary>
    private static bool Check_OrderMustHaveBillingAddress_Condition(OrderContext context)
    {
        return context?.Order.BillingAddressId != null;
    }

    /// <summary>A submitted order must not cause the customer's outstanding balance to exceed their credit limit
</summary>
    private static bool Check_CustomerCreditLimitNotExceeded_Condition(OrderContext context)
    {
        return context?.Order.Total <= context?.Customer.AvailableCredit;
    }

}